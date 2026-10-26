use super::*;
use download_engine::transfer_activity::SegmentState;

#[tokio::test]
async fn live_media_progress_tracks_partial_bytes_and_clears_on_pause_and_completion() {
    let server = Server::new().await;
    let first = ts(100, 1);
    let second = ts(20_000, 2);
    let third = ts(100, 3);
    server.set(
        "/video.m3u8",
        Reply::bytes(media(&["first.ts", "second.ts", "third.ts"])),
    );
    server.set("/first.ts", Reply::bytes(&first));
    server.set("/second.ts", Reply::bytes(&second));
    server.set("/third.ts", Reply::bytes(&third));
    let selected = playlist(&server, "/video.m3u8").await;
    let (_temp, store) = fixture();
    let job = enqueue(&store, &selected);
    let id = job.id.clone();
    let cancel = CancellationToken::new();
    let worker = tokio::spawn(engine::run(
        store.clone(),
        job,
        cancel.clone(),
        Limiter::new(&Limits {
            global: Some(64 * 1024),
            ..Default::default()
        })
        .unwrap(),
    ));
    let live = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(live) = store.activity.media(&id)
                && live.index == 1
                && live.downloaded > 0
            {
                break Some(live);
            }
            if worker.is_finished() {
                break None;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    // Always stop our worker before asserting, including on a failed observation.
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(5), worker)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let live = live
        .expect("Timed out observing live media progress")
        .expect("Worker exited without live media progress");
    assert_eq!(live.total, Some(second.len() as u64));
    assert!(live.downloaded < second.len() as u64);
    assert_eq!(live.state, SegmentState::ReceivingData);
    assert_eq!(store.activity.get(&id), None);
    assert_eq!(store.activity.media(&id), None);
    let paused = store.get(&id).unwrap().unwrap();
    assert_eq!(paused.status, Status::Paused);
    assert_eq!(paused.hls.as_ref().unwrap().completed.len(), 1);
    assert_eq!(server.count("/third.ts"), 0);
    assert!(
        store.activity.segments(&id).is_empty(),
        "Media parts must not pretend to be file ranges"
    );

    let completed = run(&store, paused).await;
    assert_eq!(completed.status, Status::Complete, "{:?}", completed.error);
    assert_eq!(
        std::fs::read(completed.saved_path.unwrap()).unwrap(),
        [first, second, third].concat()
    );
    assert_eq!(server.count("/first.ts"), 1);
    assert_eq!(server.count("/second.ts"), 2);
    assert_eq!(store.activity.get(&id), None);
    assert_eq!(store.activity.media(&id), None);
}
