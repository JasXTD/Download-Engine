use super::*;
use download_engine::categories::Category;

#[tokio::test]
async fn categorized_download_preserves_collisions_and_publishes_exact_bytes() {
    let fixture = Fixture::new(300_000, 0, false).await;
    let (_temp, store) = setup();
    let job = store
        .new_download(
            &fixture.url,
            Some("guide.pdf"),
            None,
            "category-collision".into(),
        )
        .unwrap();
    let destination = store.paths.downloads.join("Documents");
    assert_eq!(job.directory, destination);
    std::fs::create_dir_all(&destination).unwrap();
    std::fs::write(destination.join("guide.pdf"), b"original document").unwrap();
    store.enqueue(&job).unwrap();
    let done = run(&store, job).await;
    assert_eq!(done.status, Status::Complete, "{:?}", done.error);
    let published = done.saved_path.unwrap();
    assert_eq!(published.parent(), Some(destination.as_path()));
    assert_ne!(published, destination.join("guide.pdf"));
    assert_eq!(std::fs::read(&published).unwrap(), *fixture.data);
    assert_eq!(
        std::fs::read(destination.join("guide.pdf")).unwrap(),
        b"original document"
    );
    #[cfg(windows)]
    assert!(
        std::fs::read_to_string(format!("{}:Zone.Identifier", published.display()))
            .unwrap()
            .contains("ZoneId=3")
    );
}

#[tokio::test]
async fn automatic_capture_freezes_the_prepared_folder_across_rule_changes_and_reopen() {
    let fixture = Fixture::new(300_000, 0, false).await;
    let (root, store) = setup();
    let mut request = prepare_request(&fixture, "capture:folders");
    request["filename"] = "guide.pdf".into();
    let prepared = capture_request(&store, request.clone()).await;
    assert!(prepared.ok, "{}", prepared.message);
    let mut rules = store.download_folders().unwrap();
    rules
        .overrides
        .insert(Category::Documents, root.path().join("changed folder"));
    store.save_download_folders(&rules).unwrap();
    let reopened = Store::open(store.paths.clone()).unwrap();
    assert_eq!(
        capture_request(&reopened, request).await.job_id,
        prepared.job_id
    );
    assert_eq!(
        fixture.requests.lock().unwrap().len(),
        1,
        "A retry should reuse preflight"
    );
    assert!(store.list().unwrap().is_empty());
    reopened.commit_capture("capture:folders").unwrap();
    let job = reopened
        .get(prepared.job_id.as_ref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(job.directory, store.paths.downloads.join("Documents"));
    let done = run(&reopened, job).await;
    assert_eq!(done.status, Status::Complete, "{:?}", done.error);
    assert_eq!(
        std::fs::read(done.saved_path.unwrap()).unwrap(),
        *fixture.data
    );
    assert!(!root.path().join("changed folder").exists());
}

#[tokio::test]
async fn categorized_resume_keeps_original_folder_after_rules_change() {
    let fixture = Fixture::pausable_range(2_000_000, 0).await;
    let (_root, store) = setup();
    let mut job = store
        .new_download(
            &fixture.url,
            Some("bundle.zip"),
            None,
            "category-resume".into(),
        )
        .unwrap();
    job.status = Status::Downloading;
    let job = store.enqueue(&job).unwrap();
    let paused = pause_midway(&store, job, &fixture).await;
    assert!(paused.downloaded > 0);
    let mut rules = store.download_folders().unwrap();
    rules.organize = false;
    store.save_download_folders(&rules).unwrap();
    let reopened = Store::open(store.paths.clone()).unwrap();
    let done = run(&reopened, reopened.get(&paused.id).unwrap().unwrap()).await;
    assert_eq!(done.status, Status::Complete, "{:?}", done.error);
    assert_eq!(done.directory, store.paths.downloads.join("Compressed"));
    assert_eq!(
        std::fs::read(done.saved_path.unwrap()).unwrap(),
        *fixture.data
    );
}

#[tokio::test]
async fn invalid_destination_reports_failure_without_overwriting_the_existing_file() {
    let fixture = Fixture::new(32000, 0, false).await;
    let (root, store) = setup();
    let blocked = root.path().join("not-a-directory");
    std::fs::write(&blocked, b"preserve").unwrap();
    let mut rules = store.download_folders().unwrap();
    rules.overrides.insert(Category::Documents, blocked.clone());
    store.save_download_folders(&rules).unwrap();
    let job = store
        .new_download(
            &fixture.url,
            Some("guide.pdf"),
            None,
            "category-failure".into(),
        )
        .unwrap();
    store.enqueue(&job).unwrap();
    let done = run(&store, job).await;
    assert_eq!(done.status, Status::Failed);
    assert!(
        done.error
            .unwrap()
            .contains("Cannot create the download folder")
    );
    assert_eq!(std::fs::read(&blocked).unwrap(), b"preserve");
    assert!(fixture.requests.lock().unwrap().is_empty());
}
