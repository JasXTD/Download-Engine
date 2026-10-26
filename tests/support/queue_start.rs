use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_main_starts_and_immediate_resume_reuses_saved_bytes_without_another_job() {
    let fixture = Fixture::pausable_range(4 * 1024 * 1024, 2).await;
    let (_root, store) = setup();
    store.set_queue_running("main", false).unwrap();
    let scheduler = Scheduler::start(store.clone()).unwrap();
    let job = Job::new(
        &fixture.url,
        Some("fixture.bin"),
        store.paths.downloads.clone(),
        uuid::Uuid::new_v4().to_string(),
    )
    .unwrap();
    store.enqueue(&job).unwrap();
    wait_until(&store, || {
        let saved = store.get(&job.id).unwrap().unwrap();
        assert_ne!(saved.status, Status::Failed, "{:?}", saved.error);
        saved.downloaded >= 128 * 1024
    })
    .await;
    assert!(store.queues().unwrap()[0].running);
    scheduler.send(Command::Pause(job.id.clone())).unwrap();
    wait_until(&store, || {
        store.get(&job.id).unwrap().unwrap().status == Status::Paused
    })
    .await;
    let paused = store.get(&job.id).unwrap().unwrap();
    assert!(paused.downloaded > 0 && paused.downloaded < fixture.data.len() as u64);
    let plan = paused.plan.as_ref().unwrap();
    let parts = store.paths.parts(&job.id).unwrap();
    let mut expected_ranges = Vec::new();
    let mut saved_bytes = 0;
    for (index, segment) in plan.segments.iter().enumerate() {
        let saved = std::fs::metadata(parts.join(format!("segment-{index}.part")))
            .unwrap()
            .len();
        saved_bytes += saved;
        if saved < segment.end - segment.start + 1 {
            expected_ranges.push(format!(
                "range: bytes={}-{}",
                segment.start + saved,
                segment.end
            ));
        }
    }
    assert_eq!(saved_bytes, paused.downloaded);
    store.set_queue_running("main", false).unwrap();
    fixture.requests.lock().unwrap().clear();
    fixture.release_body();
    scheduler
        .send(Command::StartImmediately(job.id.clone()))
        .unwrap();
    wait_until(&store, || {
        store.get(&job.id).unwrap().unwrap().status == Status::Complete
    })
    .await;
    let complete = store.get(&job.id).unwrap().unwrap();
    assert_eq!(store.list().unwrap().len(), 1);
    assert_eq!(complete.id, paused.id);
    assert_eq!(complete.name, paused.name);
    assert_eq!(complete.directory, paused.directory);
    assert_eq!(
        std::fs::read(complete.saved_path.unwrap()).unwrap(),
        *fixture.data
    );
    assert!(
        !store.queues().unwrap()[0].running,
        "Immediate resume must not start the whole queue"
    );
    let mut actual_ranges = fixture
        .requests
        .lock()
        .unwrap()
        .iter()
        .flat_map(|request| request.lines().map(str::to_ascii_lowercase))
        .filter(|line| line.starts_with("range: ") && line != "range: bytes=0-0")
        .collect::<Vec<_>>();
    actual_ranges.sort();
    expected_ranges.sort();
    assert_eq!(
        actual_ranges, expected_ranges,
        "Resume must request only each segment's remaining bytes, after its identity probe"
    );
    tokio::task::spawn_blocking(move || drop(scheduler))
        .await
        .unwrap();
}
