use super::*;

fn add_unnamed(store: &Store, url: &str) -> Job {
    let mut job = store
        .new_download(url, None, None, uuid::Uuid::new_v4().to_string())
        .unwrap();
    job.status = Status::Downloading;
    store.enqueue(&job).unwrap()
}
fn set_header(fixture: &Fixture, value: &str) {
    *fixture.disposition.lock().unwrap() = vec![value.to_owned()];
}

#[tokio::test]
async fn server_filenames_work_for_ranges_fallbacks_empty_files_and_redirects() {
    for (size, mode, redirect) in [
        (32_768, 0, false),
        (9 * 1024 * 1024, 0, false),
        (1024, 1, false),
        (1024, 5, false),
        (0, 0, false),
        (1024, 0, true),
    ] {
        let fixture = Fixture::new(size, mode, false).await;
        set_header(
            &fixture,
            "attachment; filename=fallback.pdf; filename*=UTF-8''%E6%97%85%E8%A1%8C.pdf",
        );
        let (_root, store) = setup();
        let url = if redirect {
            fixture.url.replace("/fixture.bin", "/redirect")
        } else {
            fixture.url.clone()
        };
        let job = add_unnamed(&store, &url);
        let folder = job.directory.clone();
        let done = run(&store, job).await;
        assert_eq!(done.status, Status::Complete, "{:?}", done.error);
        assert_eq!(done.name, "旅行.pdf");
        assert!(!done.server_name_pending);
        assert_eq!(
            done.directory, folder,
            "The destination chosen at add time must stay fixed"
        );
        assert_eq!(
            done.category(),
            download_engine::categories::Category::Documents
        );
        let path = done.saved_path.unwrap();
        assert_eq!(path.file_name().unwrap(), "旅行.pdf");
        assert_eq!(path.parent(), Some(folder.as_path()));
        assert_eq!(std::fs::read(&path).unwrap(), *fixture.data);
        #[cfg(windows)]
        assert!(
            std::fs::read_to_string(format!("{}:Zone.Identifier", path.display()))
                .unwrap()
                .contains("ZoneId=3")
        );
        if size >= 9 * 1024 * 1024 {
            assert_eq!(done.plan.unwrap().segments.len(), 4);
        }
    }
}

#[tokio::test]
async fn server_filenames_preserve_explicit_browser_and_legacy_names() {
    let fixture = Fixture::new(2048, 0, false).await;
    set_header(&fixture, "attachment; filename=server.exe");
    let (_root, store) = setup();
    for explicit in ["my-report.pdf", "fixture.bin"] {
        let job = store
            .new_download(
                &fixture.url,
                Some(explicit),
                None,
                uuid::Uuid::new_v4().to_string(),
            )
            .unwrap();
        assert!(!job.server_name_pending);
        store.enqueue(&job).unwrap();
        assert_eq!(run(&store, job).await.name, explicit);
    }
    let request = serde_json::json!({"action":"add","request_id":"server-name-browser","url":fixture.url,"filename":"browser-report.pdf"});
    let result = native::handle(&store, &serde_json::to_vec(&request).unwrap());
    assert!(result.ok, "{}", result.message);
    let job = store
        .get(result.job_id.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(run(&store, job).await.name, "browser-report.pdf");
    let job = add_unnamed(&store, &fixture.url);
    let mut old = serde_json::to_value(&job).unwrap();
    old.as_object_mut().unwrap().remove("server_name_pending");
    let conn = rusqlite::Connection::open(store.paths.data.join("queue.sqlite3")).unwrap();
    conn.execute(
        "UPDATE jobs SET data=?1 WHERE id=?2",
        rusqlite::params![old.to_string(), job.id],
    )
    .unwrap();
    drop(conn);
    let reopened = Store::open(store.paths.clone()).unwrap();
    let legacy = reopened.get(&job.id).unwrap().unwrap();
    assert!(!legacy.server_name_pending);
    assert_eq!(run(&reopened, legacy).await.name, "fixture.bin");
}

#[tokio::test]
async fn server_filenames_ignore_bad_headers_and_never_overwrite_or_escape() {
    let fixture = Fixture::new(2048, 1, false).await;
    let (_root, store) = setup();
    for values in [
        vec!["attachment; filename=first.pdf; filename=second.exe"],
        vec![
            "attachment; filename=first.pdf",
            "attachment; filename=second.exe",
        ],
        vec!["attachment; filename*=UTF-8''bad%FF"],
        vec![],
    ] {
        *fixture.disposition.lock().unwrap() = values.iter().map(|s| s.to_string()).collect();
        let job = run(&store, add_unnamed(&store, &fixture.url)).await;
        assert_eq!(job.status, Status::Complete, "{:?}", job.error);
        assert_eq!(job.name, "fixture.bin");
        assert!(!job.server_name_pending);
    }
    set_header(&fixture, "attachment; filename*=UTF-8''..%2F..%2FCON.txt");
    let job = add_unnamed(&store, &fixture.url);
    let folder = job.directory.clone();
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("_CON.txt"), b"keep original").unwrap();
    let job = run(&store, job).await;
    assert_eq!(job.status, Status::Complete, "{:?}", job.error);
    assert_eq!(
        job.saved_path.as_deref(),
        Some(folder.join("_CON (1).txt").as_path())
    );
    assert_eq!(
        std::fs::read(folder.join("_CON.txt")).unwrap(),
        b"keep original"
    );
    assert_eq!(
        std::fs::read(job.saved_path.unwrap()).unwrap(),
        *fixture.data
    );
}

#[tokio::test]
async fn server_filenames_stay_fixed_after_pause_reopen_retry_and_restart() {
    let fixture = Fixture::pausable_range(2_000_000, 0).await;
    set_header(&fixture, "attachment; filename=first.zip");
    let (_root, store) = setup();
    let paused = pause_midway(&store, add_unnamed(&store, &fixture.url), &fixture).await;
    assert_eq!(paused.name, "first.zip");
    assert!(!paused.server_name_pending);
    set_header(&fixture, "attachment; filename=changed.exe");
    fixture.release_body();
    let reopened = Store::open(store.paths.clone()).unwrap();
    let done = run(&reopened, reopened.get(&paused.id).unwrap().unwrap()).await;
    assert_eq!(done.status, Status::Complete, "{:?}", done.error);
    assert_eq!(done.name, "first.zip");
    assert_eq!(
        std::fs::read(done.saved_path.unwrap()).unwrap(),
        *fixture.data
    );

    // An interrupted request must retain its first accepted name too.
    let fixture = Fixture::new(100_000, 4, false).await;
    set_header(&fixture, "attachment; filename=retry.zip");
    let failed = run(&store, add_unnamed(&store, &fixture.url)).await;
    assert_eq!(failed.status, Status::Queued); // automatic retry waiting
    assert_eq!(failed.name, "retry.zip");
    assert!(!failed.server_name_pending);
    fixture.mode.store(0, Ordering::SeqCst);
    set_header(&fixture, "attachment; filename=retry-changed.exe");
    let done = run(&store, failed).await;
    assert_eq!(done.status, Status::Complete, "{:?}", done.error);
    assert_eq!(done.name, "retry.zip");
    let mut restarted = done;
    restarted.status = Status::Downloading;
    restarted.plan = None;
    restarted.saved_path = None;
    restarted.downloaded = 0;
    let done = run(&store, restarted).await;
    assert_eq!(done.name, "retry.zip");
    assert!(done.saved_path.unwrap().ends_with("retry (1).zip"));
}

#[tokio::test]
async fn server_filenames_do_not_accept_error_response_metadata() {
    let fixture = Fixture::new(512, 7, false).await;
    set_header(&fixture, "attachment; filename=error.exe");
    let (_root, store) = setup();
    let queued = run(&store, add_unnamed(&store, &fixture.url)).await;
    assert_eq!(queued.status, Status::Queued);
    assert!(queued.server_name_pending);
    fixture.mode.store(0, Ordering::SeqCst);
    set_header(&fixture, "attachment; filename=good.pdf");
    let done = run(&store, queued).await;
    assert_eq!(done.name, "good.pdf");
    assert_eq!(done.status, Status::Complete);
}
