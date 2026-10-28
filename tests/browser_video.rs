#[path = "../src/desktop/fixture_database.rs"]
mod fixture_database;
use download_engine::{
    browser_batch::Destination,
    browser_video::{Item, Kind, Review},
    native,
    paths::AppPaths,
    store::Store,
};
use serde_json::json;

fn fixture() -> (tempfile::TempDir, Store) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    store.set_queue_running("main", false).unwrap();
    (temp, store)
}
fn id() -> String {
    format!("video:{}", uuid::Uuid::new_v4())
}
fn review() -> Review {
    Review {
        public_only: false,
        page_url: "https://example.test/watch".into(),
        page_title: "A video".into(),
        truncated: false,
        items: vec![
            Item {
                url: "https://cdn.example.test/video.mp4?token=private".into(),
                filename: "video.mp4".into(),
                label: "Video".into(),
                kind: Kind::File,
                size: Some(42),
            },
            Item {
                url: "https://cdn.example.test/master.m3u8".into(),
                filename: "Stream".into(),
                label: "Choose quality".into(),
                kind: Kind::Hls,
                size: None,
            },
        ],
    }
}
fn send(store: &Store, id: &str, review: &Review) -> native::Reply {
    native::handle(
        store,
        &serde_json::to_vec(
            &json!({"action":"review_browser_video","request_id":id,"review":review}),
        )
        .unwrap(),
    )
}

#[test]
fn native_review_is_durable_idempotent_and_never_a_download() {
    let (_temp, store) = fixture();
    let id = id();
    let review = review();
    let reply = send(&store, &id, &review);
    assert!(reply.ok, "{}", reply.message);
    assert_eq!(reply.preview_id.as_deref(), Some(id.as_str()));
    assert!(reply.job_id.is_none() && reply.job_ids.is_empty());
    assert!(store.list().unwrap().is_empty());
    let reopened = Store::open(store.paths.clone()).unwrap();
    assert_eq!(
        reopened.next_browser_video_review().unwrap().unwrap().id,
        id
    );
    assert!(send(&reopened, &id, &review).ok);
    let mut changed = review.clone();
    changed.page_title = "Other".into();
    assert!(!send(&store, &id, &changed).ok);
    store.dismiss_browser_video_review(&id).unwrap();
    assert!(store.next_browser_video_review().unwrap().is_none());
    assert!(!store.request_browser_video_review(&id, &review).unwrap());
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn native_empty_review_and_candidates_are_validated_at_the_boundary() {
    let (_temp, store) = fixture();
    let mut empty = review();
    empty.items.clear();
    assert!(send(&store, &id(), &empty).ok);
    for url in [
        "file:///C:/secret",
        "javascript:alert(1)",
        "blob:https://example.test/x",
        "https://user:password@example.test/v",
    ] {
        let mut invalid = review();
        invalid.items[0].url = url.into();
        assert!(!send(&store, &id(), &invalid).ok);
        invalid = review();
        invalid.page_url = url.into();
        assert!(!send(&store, &id(), &invalid).ok);
    }
    let mut invalid = review();
    invalid.items.push(invalid.items[0].clone());
    assert!(!send(&store, &id(), &invalid).ok);
    invalid = review();
    invalid.items[1].kind = Kind::File;
    assert!(!send(&store, &id(), &invalid).ok);
    invalid = review();
    invalid.items[0].size = Some(u64::MAX);
    assert!(!send(&store, &id(), &invalid).ok);
    invalid = review();
    invalid.items.resize(1001, invalid.items[0].clone());
    assert!(!send(&store, &id(), &invalid).ok);
    let mut extra = json!({"action":"review_browser_video","request_id":id(),"review":review()});
    extra["review"]["cookies"] = json!(["secret"]);
    assert!(!native::handle(&store, &serde_json::to_vec(&extra).unwrap()).ok);
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn desktop_adds_only_selected_files_and_consumes_review_atomically() {
    let (_temp, store) = fixture();
    let id = id();
    store.request_browser_video_review(&id, &review()).unwrap();
    for indices in [vec![], vec![8], vec![0, 0], vec![0, 1]] {
        assert!(
            store
                .add_browser_video_selection(&id, &indices, "main", Destination::Video)
                .is_err()
        );
        assert!(store.list().unwrap().is_empty());
        assert!(store.next_browser_video_review().unwrap().is_some());
    }
    assert!(
        store
            .add_browser_video_selection(&id, &[0], "missing", Destination::Video)
            .is_err()
    );
    let ids = store
        .add_browser_video_selection(&id, &[0], "main", Destination::Video)
        .unwrap();
    assert_eq!(ids.len(), 1);
    assert_eq!(store.list().unwrap().len(), 1);
    assert!(store.next_browser_video_review().unwrap().is_none());
    assert_eq!(
        store
            .add_browser_video_selection(&id, &[0], "main", Destination::Video)
            .unwrap(),
        ids
    );
    let job = store.get(&ids[0]).unwrap().unwrap();
    assert!(job.hls.is_none());
    assert_eq!(job.name, "video.mp4");
    assert_eq!(job.url, review().items[0].url);
    // The page is kept so its origin is sent as the Referer.
    assert_eq!(
        job.source_page.as_deref(),
        Some("https://example.test/watch")
    );
    assert_eq!(store.job_queue(&job.id).unwrap().as_deref(), Some("main"));
}

#[test]
fn stream_choice_only_opens_quality_preview_and_expired_reviews_cannot_be_used() {
    let (_temp, store) = fixture();
    let first = id();
    store
        .request_browser_video_review(&first, &review())
        .unwrap();
    assert!(store.take_browser_video_stream(&first, 0).is_err());
    assert_eq!(
        store.take_browser_video_stream(&first, 1).unwrap(),
        review().items[1].url
    );
    assert!(store.list().unwrap().is_empty());
    assert!(store.next_browser_video_review().unwrap().is_none());
    let second = id();
    store
        .request_browser_video_review(&second, &review())
        .unwrap();
    let conn = crate::fixture_database::open(store.paths.data.join("queue.sqlite3")).unwrap();
    conn.execute(
        "UPDATE browser_video_reviews SET created=?1 WHERE request_id=?2",
        rusqlite::params![chrono::Utc::now().timestamp() - 1801, second],
    )
    .unwrap();
    assert!(store.take_browser_video_stream(&second, 1).is_err());
    assert!(
        store
            .add_browser_video_selection(&second, &[0], "main", Destination::Automatic)
            .is_err()
    );
    assert!(store.next_browser_video_review().unwrap().is_none());
    let payloads: i64 = conn
        .query_row(
            "SELECT count(*) FROM browser_video_reviews WHERE data IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(payloads, 0);
}

#[test]
fn pending_reviews_are_bounded_and_concurrent_retries_share_one_request() {
    let (_temp, store) = fixture();
    let first = id();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    store
                        .request_browser_video_review(&first, &review())
                        .unwrap()
                })
            })
            .collect();
        for handle in handles {
            assert!(handle.join().unwrap());
        }
    });
    for _ in 0..9 {
        store
            .request_browser_video_review(&id(), &review())
            .unwrap();
    }
    assert!(
        store
            .request_browser_video_review(&id(), &review())
            .is_err()
    );
    store.dismiss_browser_video_review(&first).unwrap();
    assert!(
        store
            .request_browser_video_review(&id(), &review())
            .unwrap()
    );
}

#[test]
fn real_native_host_frames_review_receipts_without_starting_downloads_or_logging_urls() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let (temp, store) = fixture();
    // Simulate an already-running desktop in this isolated profile. The real
    // connector must acknowledge the saved review without opening another app.
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(store.paths.data.join("desktop.lock"))
        .unwrap();
    fs2::FileExt::lock_exclusive(&lock).unwrap();
    let id = id();
    let bytes = serde_json::to_vec(
        &json!({"action":"review_browser_video","request_id":id,"review":review()}),
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_de-host"))
        .env("IDM_EX_HOME", temp.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    for _ in 0..2 {
        input
            .write_all(&(bytes.len() as u32).to_le_bytes())
            .unwrap();
        input.write_all(&bytes).unwrap();
    }
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut stdout = std::io::Cursor::new(output.stdout);
    for _ in 0..2 {
        let reply: native::Reply =
            serde_json::from_slice(&native::read_frame(&mut stdout).unwrap().unwrap()).unwrap();
        assert!(reply.ok, "{}", reply.message);
        assert_eq!(reply.preview_id.as_deref(), Some(id.as_str()));
        assert!(reply.job_id.is_none() && reply.job_ids.is_empty());
    }
    assert!(native::read_frame(&mut stdout).unwrap().is_none());
    assert_eq!(store.next_browser_video_review().unwrap().unwrap().id, id);
    assert!(store.list().unwrap().is_empty());
    let log = std::fs::read_to_string(store.paths.data.join("logs/native-host.log")).unwrap();
    assert!(log.contains("review_video"));
    assert!(!log.contains("example.test") && !log.contains("token=private"));
}
