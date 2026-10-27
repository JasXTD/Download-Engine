use super::*;
use sha2::Digest;

pub(super) async fn wait_saved(store: &Store, id: &str, predicate: impl Fn(&Job) -> bool) -> Job {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let job = store.get(id).unwrap().unwrap();
            if predicate(&job) {
                return job;
            }
            assert!(
                !matches!(job.status, Status::Failed | Status::Complete),
                "Unexpected terminal state: {:?}",
                job.error
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Expected persisted parallel progress")
}

#[tokio::test]
async fn failed_parallel_part_joins_peers_and_retry_reuses_verified_parts() {
    let server = Server::new().await;
    let (_temp, store) = fixture();
    let names = ["0.ts", "1.ts", "2.ts", "3.ts", "4.ts", "5.ts"];
    server.set("/video.m3u8", Reply::bytes(media(&names)));
    let body = ts(100, 7);
    let failed_gate = Arc::new(tokio::sync::Semaphore::new(0));
    let peer_gate = Arc::new(tokio::sync::Semaphore::new(0));
    for (index, name) in names.iter().enumerate() {
        server.set(
            &format!("/{name}"),
            Reply {
                cut: index == 1,
                gate: match index {
                    1 => Some(failed_gate.clone()),
                    2 => Some(peer_gate.clone()),
                    _ => None,
                },
                ..Reply::bytes(&body)
            },
        );
    }
    let mut job = enqueue(&store, &playlist(&server, "/video.m3u8").await);
    job.status = Status::Downloading;
    let id = job.id.clone();
    let worker = tokio::spawn(engine::run(
        store.clone(),
        job,
        CancellationToken::new(),
        Limiter::default(),
    ));
    wait_saved(&store, &id, |job| {
        job.hls.as_ref().unwrap().completed_ahead.len() == 3
    })
    .await;
    failed_gate.add_permits(1);
    tokio::time::timeout(Duration::from_secs(5), worker)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let retry = store.get(&id).unwrap().unwrap();
    assert_eq!(retry.status, Status::Queued, "{:?}", retry.error);
    assert!(retry.retry.next_at.is_some());
    assert_eq!(retry.hls.as_ref().unwrap().completed_count(), 4);
    assert!(store.activity.media_parts(&id).is_empty());
    assert!(retry.saved_path.is_none());
    for index in [1, 2] {
        server.set(&format!("/{index}.ts"), Reply::bytes(&body));
    }
    let done = run(&store, retry).await;
    assert_eq!(done.status, Status::Complete, "{:?}", done.error);
    assert_eq!(
        std::fs::read(done.saved_path.unwrap()).unwrap(),
        body.repeat(6)
    );
    for (index, name) in names.iter().enumerate() {
        assert_eq!(
            server.count(&format!("/{name}")),
            if [1, 2].contains(&index) { 2 } else { 1 }
        );
    }
}

#[tokio::test]
async fn parallel_pause_reopen_preserves_out_of_order_parts_without_refetching() {
    let server = Server::new().await;
    let (_temp, store) = fixture();
    let names = ["0.ts", "1.ts", "2.ts", "3.ts", "4.ts", "5.ts"];
    server.set("/video.m3u8", Reply::bytes(media(&names)));
    let bodies: Vec<_> = (0..names.len()).map(|index| ts(100, index as u8)).collect();
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    for (index, name) in names.iter().enumerate() {
        server.set(
            &format!("/{name}"),
            Reply {
                gate: (index == 1).then(|| gate.clone()),
                ..Reply::bytes(&bodies[index])
            },
        );
    }
    let selected = playlist(&server, "/video.m3u8").await;
    let mut job = enqueue(&store, &selected);
    job.status = Status::Downloading;
    let id = job.id.clone();
    let token = CancellationToken::new();
    let worker = tokio::spawn(engine::run(
        store.clone(),
        job,
        token.clone(),
        Limiter::default(),
    ));
    let observed = wait_saved(&store, &id, |job| {
        job.hls.as_ref().unwrap().completed_ahead.len() == 4
    })
    .await;
    assert_eq!(observed.hls.as_ref().unwrap().completed.len(), 1);
    token.cancel();
    tokio::time::timeout(Duration::from_secs(5), worker)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let reopened = Store::open(store.paths.clone()).unwrap();
    let paused = reopened.get(&id).unwrap().unwrap();
    assert_eq!(paused.status, Status::Paused);
    assert_eq!(paused.hls.as_ref().unwrap().completed_count(), 5);
    assert!(store.activity.media_parts(&id).is_empty());
    // Reopen the response gate for the new request; the cancelled socket cannot
    // own or write the saved part after engine::run has returned.
    server.set("/1.ts", Reply::bytes(&bodies[1]));
    let done = run(&reopened, paused).await;
    assert_eq!(done.status, Status::Complete, "{:?}", done.error);
    assert_eq!(
        std::fs::read(done.saved_path.unwrap()).unwrap(),
        bodies.concat()
    );
    for (index, name) in names.iter().enumerate() {
        assert_eq!(
            server.count(&format!("/{name}")),
            if index == 1 { 2 } else { 1 }
        );
    }
    assert!(done.hls.unwrap().completed_ahead.is_empty());
    assert_eq!(reopened.list().unwrap().len(), 1);
}

#[tokio::test]
async fn video_worker_limit_and_pause_join_all_active_writers() {
    for (limit, site_limit) in [
        (1, None),
        (2, None),
        (4, None),
        (16, None),
        (16, Some(1)),
        (16, Some(2)),
    ] {
        let server = Server::new().await;
        let (_temp, store) = fixture();
        let settings = store.connection_settings().unwrap();
        let mut updated = settings.clone();
        updated.default_connections = limit;
        if let Some(connections) = site_limit {
            updated.sites.push(download_engine::connections::Site {
                host: "127.0.0.1".into(),
                subdomains: false,
                connections,
            });
        }
        store.save_connection_settings(&settings, updated).unwrap();
        let names: Vec<_> = (0..10).map(|i| format!("{i}.ts")).collect();
        server.set(
            "/video.m3u8",
            Reply::bytes(media(&names.iter().map(String::as_str).collect::<Vec<_>>())),
        );
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        for (index, name) in names.iter().enumerate() {
            server.set(
                &format!("/{name}"),
                Reply {
                    gate: (index > 0).then(|| gate.clone()),
                    ..Reply::bytes(ts(100, index as u8))
                },
            );
        }
        let selected = playlist(&server, "/video.m3u8").await;
        let mut job = enqueue(&store, &selected);
        job.status = Status::Downloading;
        let id = job.id.clone();
        let token = CancellationToken::new();
        let worker = tokio::spawn(engine::run(
            store.clone(),
            job,
            token.clone(),
            Limiter::default(),
        ));
        let expected = usize::from(site_limit.unwrap_or(limit).min(4));
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if store.activity.media_parts(&id).len() == expected
                    && server.count(&format!("/{expected}.ts")) == 1
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            names
                .iter()
                .skip(1)
                .map(|name| server.count(&format!("/{name}")))
                .sum::<usize>(),
            expected
        );
        token.cancel();
        tokio::time::timeout(Duration::from_secs(5), worker)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(store.get(&id).unwrap().unwrap().status, Status::Paused);
        assert!(store.activity.media_parts(&id).is_empty());
        let root = store.paths.parts(&id).unwrap();
        // Exclusive guarded handles have been released before pause returns.
        for index in 1..=expected {
            let path = root.join(format!("hls-{index}.part"));
            if path.exists() {
                std::fs::remove_file(path).unwrap();
            }
        }
    }
}

#[tokio::test]
async fn corrupted_out_of_order_receipt_is_rejected_before_any_new_media() {
    let server = Server::new().await;
    let (_temp, store) = fixture();
    server.set(
        "/video.m3u8",
        Reply::bytes(media(&["0.ts", "1.ts", "2.ts"])),
    );
    for i in 0..3 {
        server.set(&format!("/{i}.ts"), Reply::bytes(ts(100, i)));
    }
    let selected = playlist(&server, "/video.m3u8").await;
    let mut job = enqueue(&store, &selected);
    let root = store.paths.parts(&job.id).unwrap();
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("hls-2.part"), ts(100, 99)).unwrap();
    job.hls.as_mut().unwrap().completed_ahead.insert(
        2,
        hls::CompletedPart {
            size: 18800,
            sha256: format!("{:x}", sha2::Sha256::digest(ts(100, 2))),
        },
    );
    store.save(&job).unwrap();
    let failed = run(&store, job).await;
    assert_eq!(failed.status, Status::Failed);
    assert!(failed.error.unwrap().contains("changed"));
    for i in 0..3 {
        assert_eq!(server.count(&format!("/{i}.ts")), 0);
    }
}
