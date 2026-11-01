use super::*;
fn fixture() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(crate::paths::AppPaths {
        data: dir.path().join("data"),
        downloads: dir.path().join("downloads"),
    })
    .unwrap();
    (dir, store)
}
fn target(n: usize) -> Target {
    Target::Magnet(format!("magnet:?xt=urn:btih:{n:040x}&dn=Fixture"))
}
#[test]
fn parser_accepts_unicode_paths_and_valid_magnets_only() {
    let (dir, _) = fixture();
    assert!(
        matches!(Target::parse(OsStr::new("sample 文件.TORRENT"), dir.path()).unwrap(), Target::TorrentFile(p) if p==dir.path().join("sample 文件.TORRENT"))
    );
    assert!(Target::parse(OsStr::new(target(1).text().unwrap()), dir.path()).is_ok());
    for text in [
        "",
        "--antivirus-scan-request",
        "https://example.com/a.torrent",
        "magnet:?dn=bad",
        "a.exe",
        "a\ntorrent",
        "--browser-setup",
    ] {
        assert!(
            Target::parse(OsStr::new(text), dir.path()).is_err(),
            "{text}"
        );
    }
    #[cfg(windows)]
    assert!(Target::parse(OsStr::new("file:///a.torrent"), dir.path()).is_err());
    #[cfg(unix)]
    {
        let path = dir.path().join("sample 文件.torrent");
        let url = reqwest::Url::from_file_path(&path).unwrap();
        assert!(
            matches!(Target::parse(OsStr::new(url.as_str()), dir.path()).unwrap(), Target::TorrentFile(p) if p == path)
        );
        for value in [
            "file://other-host/a.torrent",
            "file:///a.torrent?command=x",
            "file:///a.torrent#x",
            "file:///a.exe",
        ] {
            assert!(
                Target::parse(OsStr::new(value), dir.path()).is_err(),
                "{value}"
            );
        }
    }
}
#[test]
fn receiving_requires_review_and_survives_profile_reopening() {
    let (_dir, store) = fixture();
    let ids = store
        .submit_external_opens(&[target(1), target(2)])
        .unwrap();
    assert!(!store.external_opens_received(&ids).unwrap());
    let inbox = store.receive_external_opens().unwrap();
    assert_eq!(inbox.count, 2);
    assert_eq!(inbox.newly_received, 2);
    assert_eq!(inbox.next.unwrap().id, ids[0].id);
    assert!(store.external_opens_received(&ids).unwrap());
    assert!(store.list().unwrap().is_empty());
    assert!(!store.torrent_settings().unwrap().enabled);
    let reopened = Store::open(store.paths.clone()).unwrap();
    assert_eq!(reopened.receive_external_opens().unwrap().count, 2);
    reopened.dismiss_external_open(&ids[0].id).unwrap();
    assert_eq!(
        reopened.receive_external_opens().unwrap().next.unwrap().id,
        ids[1].id
    );
    assert!(reopened.external_opens_received(&ids).unwrap());
}
#[test]
fn repeated_magnets_are_one_review_and_timeout_does_not_remove_another_launch() {
    let (_dir, store) = fixture();
    let original = store.submit_external_opens(&[target(1)]).unwrap();
    let duplicate = store
        .submit_external_opens(&[Target::Magnet(format!(
            "{}&tr=https%3A%2F%2Fexample.com",
            target(1).text().unwrap()
        ))])
        .unwrap();
    assert_eq!(original[0].id, duplicate[0].id);
    assert!(!duplicate[0].owned);
    assert!(
        !store
            .withdraw_unreceived_external_opens(&duplicate)
            .unwrap()
    );
    assert_eq!(store.receive_external_opens().unwrap().count, 1);
    assert!(store.withdraw_unreceived_external_opens(&original).unwrap());
    assert_eq!(store.receive_external_opens().unwrap().count, 1);
}
#[test]
fn timeout_removes_only_unreceived_owned_requests() {
    let (_dir, store) = fixture();
    let ids = store.submit_external_opens(&[target(1)]).unwrap();
    assert!(!store.withdraw_unreceived_external_opens(&ids).unwrap());
    assert_eq!(store.receive_external_opens().unwrap().count, 0);
}
#[test]
fn queue_limits_and_invalid_inputs_roll_back_the_whole_launch() {
    let (_dir, store) = fixture();
    let targets: Vec<_> = (0..MAX_PENDING - 1).map(target).collect();
    store.submit_external_opens(&targets).unwrap();
    assert!(
        store
            .submit_external_opens(&[target(99), target(100)])
            .is_err()
    );
    assert_eq!(
        store.receive_external_opens().unwrap().count,
        MAX_PENDING - 1
    );
    assert!(
        store
            .submit_external_opens(&[target(99), Target::Magnet("magnet:?xt=bad".into())])
            .is_err()
    );
    assert_eq!(
        store.receive_external_opens().unwrap().count,
        MAX_PENDING - 1
    );
}
#[test]
fn old_pending_requests_expire_without_creating_downloads() {
    let (_dir, store) = fixture();
    store.submit_external_opens(&[target(1)]).unwrap();
    store
        .connect()
        .unwrap()
        .execute("UPDATE external_open_requests SET created=0", [])
        .unwrap();
    assert_eq!(store.receive_external_opens().unwrap().count, 0);
    assert!(store.list().unwrap().is_empty());
}
#[test]
fn corrupted_pending_request_is_reported_once_and_does_not_block_the_next() {
    let (_dir, store) = fixture();
    let requests = store
        .submit_external_opens(&[target(1), target(2)])
        .unwrap();
    store
        .connect()
        .unwrap()
        .execute(
            "UPDATE external_open_requests SET data='invalid json' WHERE id=?1",
            [&requests[0].id],
        )
        .unwrap();
    assert!(store.receive_external_opens().is_err());
    let inbox = store.receive_external_opens().unwrap();
    assert_eq!(inbox.count, 1);
    assert_eq!(inbox.next.unwrap().id, requests[1].id);
}
