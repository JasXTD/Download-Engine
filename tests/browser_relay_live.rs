//! Driven only by browser_video_relay_live.cjs with an isolated real browser.
use download_engine::{
    bandwidth::{Limiter, Limits},
    browser_relay::Session,
    engine, hls,
    model::Status,
    paths::AppPaths,
    proxy::Route,
    queue::MAIN_QUEUE,
    store::Store,
};
use std::{path::PathBuf, time::Duration};
use tokio_util::sync::CancellationToken;

#[tokio::test]
#[ignore = "Requires the explicit isolated real-browser fixture"]
async fn real_browser_download_pause_resume() {
    let root =
        PathBuf::from(std::env::var_os("DE_BROWSER_RELAY_FIXTURE").expect("fixture directory"));
    assert!(root.is_absolute());
    let session: Session =
        serde_json::from_slice(&std::fs::read(root.join("session.json")).unwrap()).unwrap();
    let page = reqwest::Url::parse(&session.page).unwrap();
    assert_eq!(page.host_str(), Some("127.0.0.1"));
    assert_eq!(page.scheme(), "http");
    let base = page.origin().ascii_serialization();
    let store = Store::open(AppPaths {
        data: root.join("app/data"),
        downloads: root.join("app/downloads"),
    })
    .unwrap();
    let cancel = CancellationToken::new();
    let source = format!("{base}/index.m3u8");
    // Demonstrate the failure first using the unchanged desktop HTTP transport.
    let error = hls::inspect_browser_via(
        &source,
        &[],
        Some(&session.page),
        None,
        false,
        &cancel,
        Route::load(&store).unwrap(),
    )
    .await
    .unwrap_err();
    assert_eq!(
        download_engine::diagnostics::Failure::error(&error).http_status,
        Some(403)
    );
    let inspected = hls::inspect_browser_relay_via(
        &source,
        &[],
        hls::BrowserContext {
            page: Some(&session.page),
            user_agent: None,
            relay: Some(&session),
        },
        false,
        &cancel,
        Route::load(&store).unwrap(),
    )
    .await
    .unwrap();
    let playlist =
        hls::preview_with_audio_via(&inspected, 0, 0, &cancel, Route::load(&store).unwrap())
            .await
            .unwrap();
    let mut job = store
        .enqueue_inspected_hls(
            &inspected,
            &playlist,
            "fixture.ts",
            None,
            MAIN_QUEUE,
            "real-browser-fixture",
        )
        .unwrap();
    job.single_connection = true;
    store.save(&job).unwrap();
    let id = job.id.clone();
    let work_store = store.clone();
    let pause = CancellationToken::new();
    let stop = pause.clone();
    let worker = tokio::spawn(async move {
        engine::run(
            work_store,
            job,
            stop,
            Limiter::new(&Limits::default()).unwrap(),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let saved = store.get(&id).unwrap().unwrap();
            assert!(
                !matches!(saved.status, Status::Failed | Status::Complete),
                "unexpected early state {:?}: {:?}",
                saved.status,
                saved.error
            );
            if saved.hls.as_ref().unwrap().completed_count() == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("first verified part");
    pause.cancel();
    tokio::time::timeout(Duration::from_secs(5), worker)
        .await
        .expect("prompt pause")
        .unwrap()
        .unwrap();
    let paused = store.get(&id).unwrap().unwrap();
    assert_eq!(paused.status, Status::Paused, "{:?}", paused.error);
    assert_eq!(paused.downloaded, 188 * 500);
    assert_eq!(paused.hls.as_ref().unwrap().completed_count(), 1);
    let reopened = Store::open(store.paths.clone()).unwrap();
    let resume = reopened.get(&id).unwrap().unwrap();
    assert!(resume.hls.as_ref().unwrap().browser_relay.is_some());
    std::fs::write(root.join("resume.ready"), b"resume").unwrap();
    tokio::time::timeout(
        Duration::from_secs(30),
        engine::run(
            reopened.clone(),
            resume,
            CancellationToken::new(),
            Limiter::new(&Limits::default()).unwrap(),
        ),
    )
    .await
    .expect("resume completion")
    .unwrap();
    let done = reopened.get(&id).unwrap().unwrap();
    assert_eq!(done.status, Status::Complete, "{:?}", done.error);
    let expected: Vec<u8> = [3, 4]
        .into_iter()
        .flat_map(|value| {
            let mut bytes = vec![value; 188 * 500];
            for packet in bytes.chunks_mut(188) {
                packet[0] = 0x47;
            }
            bytes
        })
        .collect();
    assert_eq!(std::fs::read(done.saved_path.unwrap()).unwrap(), expected);
    assert_eq!(
        reopened.list().unwrap().len(),
        1,
        "Resume created a duplicate job"
    );
    println!(
        "Verified desktop403 -> browser preview -> paused saved part -> reopened database -> exact resumed bytes; one job."
    );

    // A realistic parallel transfer, larger than the reported 99 MB failure.
    // Fast segments can accumulate browser reads much larger than an IPC frame.
    let bulk = hls::inspect_browser_relay_via(
        &format!("{base}/bulk.m3u8"),
        &[],
        hls::BrowserContext {
            page: Some(&session.page),
            user_agent: None,
            relay: Some(&session),
        },
        false,
        &cancel,
        Route::load(&store).unwrap(),
    )
    .await
    .unwrap();
    let playlist = hls::preview_with_audio_via(&bulk, 0, 0, &cancel, Route::load(&store).unwrap())
        .await
        .unwrap();
    let job = store
        .enqueue_inspected_hls(
            &bulk,
            &playlist,
            "parallel.ts",
            None,
            MAIN_QUEUE,
            "parallel-browser-fixture",
        )
        .unwrap();
    assert!(!job.single_connection);
    let id = job.id.clone();
    tokio::time::timeout(
        Duration::from_secs(180),
        engine::run(
            store.clone(),
            job,
            CancellationToken::new(),
            Limiter::new(&Limits::default()).unwrap(),
        ),
    )
    .await
    .expect("large parallel completion")
    .unwrap();
    let done = store.get(&id).unwrap().unwrap();
    assert_eq!(done.status, Status::Complete, "{:?}", done.error);
    let expected_length = 34 * 188 * 22310;
    assert_eq!(done.downloaded, expected_length);
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut expected = Sha256::new();
    for index in 0..34 {
        let mut part = vec![index + 8; 188 * 22310];
        for packet in part.chunks_mut(188) {
            packet[0] = 0x47;
        }
        expected.update(part);
    }
    let mut actual = Sha256::new();
    let mut file = std::fs::File::open(done.saved_path.unwrap()).unwrap();
    assert_eq!(file.metadata().unwrap().len(), expected_length);
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        actual.update(&buffer[..count]);
    }
    assert_eq!(actual.finalize(), expected.finalize());
    assert_eq!(store.list().unwrap().len(), 2);
    println!(
        "Verified 34 parallel segments: {expected_length} bytes, exact SHA-256, no duplicate job."
    );
}
