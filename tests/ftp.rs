use download_engine::{
    bandwidth::{Limiter, Limits},
    engine,
    model::{Job, Status},
    paths::AppPaths,
    scheduler::{Command, Scheduler},
    store::Store,
};
use sha2::{Digest, Sha256};
use std::{sync::atomic::Ordering, time::Duration};
use tokio_util::sync::CancellationToken;
#[path = "support/ftp_server.rs"]
mod fixture;
use fixture::{Mode, Server};

fn setup() -> (tempfile::TempDir, Store) {
    let home = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: home.path().join("data"),
        downloads: home.path().join("files"),
    })
    .unwrap();
    (home, store)
}
fn enqueue(store: &Store, server: &Server) -> Job {
    let job = Job::new(
        &server.url(false),
        None,
        store.paths.downloads.clone(),
        uuid::Uuid::new_v4().to_string(),
    )
    .unwrap();
    store.enqueue(&job).unwrap()
}
async fn run(store: &Store, mut job: Job) -> Job {
    job.status = Status::Downloading;
    let id = job.id.clone();
    engine::run(
        store.clone(),
        job,
        CancellationToken::new(),
        Limiter::new(&store.bandwidth_limits().unwrap()).unwrap(),
    )
    .await
    .unwrap();
    store.get(&id).unwrap().unwrap()
}
async fn until(mut check: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(15), async {
        while !check() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("fixture condition timed out");
}
fn assert_complete(job: &Job, body: &[u8]) {
    assert_eq!(job.status, Status::Complete, "{:?}", job.error);
    let path = job.saved_path.as_ref().unwrap();
    assert_eq!(std::fs::read(path).unwrap(), body);
    assert_eq!(
        job.integrity.actual.as_deref(),
        Some(format!("{:x}", Sha256::digest(body)).as_str())
    );
    #[cfg(windows)]
    assert!(
        std::fs::read_to_string(format!("{}:Zone.Identifier", path.display()))
            .unwrap()
            .contains("ZoneId=3")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn anonymous_ftp_handles_binary_empty_and_unknown_length_files_in_passive_modes() {
    for case in 0..4 {
        let server = Server::new(Mode::Plain).await;
        server.update(|c| {
            c.epsv = case != 1;
            if case == 2 {
                c.body.clear();
            }
            if case == 3 {
                c.size = false;
                c.timestamp = false;
            }
        });
        let (_home, store) = setup();
        let job = run(&store, enqueue(&store, &server)).await;
        assert_complete(&job, &server.state.config.lock().unwrap().body);
        let commands = server.commands();
        assert!(commands.contains(&"USER anonymous".to_string()));
        assert!(commands.contains(&"TYPE I".to_string()));
        assert_eq!(commands.contains(&"PASV".to_string()), case == 1);
        assert!(
            !commands
                .iter()
                .any(|s| s.starts_with("PORT ") || s.starts_with("EPRT "))
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pause_reopen_and_resume_verifies_saved_prefix_without_using_weak_timestamps() {
    let server = Server::new(Mode::Plain).await;
    server.update(|c| c.delay = Duration::from_millis(20));
    let (_home, store) = setup();
    let job = enqueue(&store, &server);
    let id = job.id.clone();
    let part = store.paths.parts(&id).unwrap().join("ftp.part");
    let cancel = CancellationToken::new();
    let worker = tokio::spawn(engine::run(
        store.clone(),
        job,
        cancel.clone(),
        Limiter::default(),
    ));
    until(|| std::fs::metadata(&part).is_ok_and(|m| m.len() >= 16 * 1024)).await;
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(2), worker)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let paused = store.get(&id).unwrap().unwrap();
    let prefix = std::fs::read(&part).unwrap();
    assert_eq!(paused.status, Status::Paused);
    assert_eq!(paused.downloaded, prefix.len() as u64);
    assert!(paused.downloaded > 0 && paused.downloaded < paused.total.unwrap());
    assert!(paused.saved_path.is_none());
    server.update(|c| c.delay = Duration::ZERO);
    let reopened = Store::open(store.paths.clone()).unwrap();
    let complete = run(&reopened, reopened.get(&id).unwrap().unwrap()).await;
    assert_complete(&complete, &server.state.config.lock().unwrap().body);
    assert!(!server.commands().iter().any(|s| s.starts_with("REST ")));
    assert!(server.state.sent.load(Ordering::SeqCst) > complete.downloaded as usize);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn changed_remote_bytes_or_metadata_never_append_to_an_old_partial_file() {
    for metadata in [false, true] {
        let server = Server::new(Mode::Plain).await;
        server.update(|c| c.cut = Some(32768));
        let (_home, store) = setup();
        let partial = run(&store, enqueue(&store, &server)).await;
        assert_eq!(partial.status, Status::Queued);
        assert!(partial.downloaded > 0);
        let path = store.paths.parts(&partial.id).unwrap().join("ftp.part");
        let before = std::fs::read(&path).unwrap();
        server.update(|c| {
            c.cut = None;
            if metadata {
                c.modified = "20261013120000.000".into();
            } else {
                c.body[0] ^= 255;
            }
        });
        let stopped = run(&store, partial).await;
        assert_eq!(stopped.status, Status::Failed, "{:?}", stopped.error);
        assert!(stopped.error.unwrap().contains("Restart"));
        assert!(stopped.saved_path.is_none());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn publisher_checksum_allows_rest_and_unsupported_rest_falls_back_to_prefix_verification() {
    for rest in [true, false] {
        let server = Server::new(Mode::Plain).await;
        server.update(|c| {
            c.cut = Some(32768);
            c.rest = rest;
        });
        let (_home, store) = setup();
        let mut job = enqueue(&store, &server);
        let body = server.state.config.lock().unwrap().body.clone();
        job.integrity.expected = Some(format!("{:x}", Sha256::digest(&body)));
        store.save(&job).unwrap();
        let partial = run(&store, job).await;
        let offset = partial.downloaded;
        assert!(offset > 0);
        server.update(|c| c.cut = None);
        let complete = run(&store, partial).await;
        assert_complete(&complete, &body);
        assert!(server.commands().contains(&format!("REST {offset}")));
        assert_eq!(
            server.state.sent.load(Ordering::SeqCst),
            body.len() + if rest { 0 } else { offset as usize }
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resume_hash_mismatch_completion_failure_and_mid_transfer_change_never_publish() {
    for case in 0..3 {
        let server = Server::new(Mode::Plain).await;
        let (_home, store) = setup();
        let mut job = enqueue(&store, &server);
        if case == 0 {
            job.integrity.expected = Some(format!(
                "{:x}",
                Sha256::digest(&server.state.config.lock().unwrap().body)
            ));
            server.update(|c| c.cut = Some(32768));
            job = run(&store, job).await;
            server.update(|c| {
                c.cut = None;
                c.body[40000] ^= 255;
            });
        } else {
            server.update(|c| {
                c.end_status = if case == 1 { 550 } else { 226 };
                c.change_after = case == 2;
            });
        }
        let stopped = run(&store, job).await;
        assert_eq!(stopped.status, Status::Failed, "{:?}", stopped.error);
        assert!(stopped.saved_path.is_none());
        assert!(!store.paths.downloads.join("fixture.bin").exists());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn protocol_bounds_and_permanent_errors_stop_while_transient_errors_have_bounded_retries() {
    for case in 0..4 {
        let server = Server::new(Mode::Plain).await;
        server.update(|c| match case {
            0 => c.low_port = true,
            1 => c.huge_reply = true,
            2 => c.require_login = true,
            _ => c.retr_status = Some(550),
        });
        let (_home, store) = setup();
        let job = run(&store, enqueue(&store, &server)).await;
        assert_eq!(job.status, Status::Failed, "{:?}", job.error);
        assert_eq!(job.retry.attempts, 0);
        assert!(!job.error.unwrap().contains("fixture-password"));
        assert!(job.saved_path.is_none());
        if case <= 1 {
            assert!(!server.commands().iter().any(|s| s.starts_with("RETR")));
        }
    }
    let server = Server::new(Mode::Plain).await;
    server.update(|c| c.retr_status = Some(450));
    let (_home, store) = setup();
    let mut job = enqueue(&store, &server);
    for attempt in 1..=download_engine::retry::MAX_RETRIES {
        job = run(&store, job).await;
        assert_eq!(job.status, Status::Queued);
        assert_eq!(job.retry.attempts, attempt);
        assert!(job.retry.next_at.is_some());
    }
    job = run(&store, job).await;
    assert_eq!(job.status, Status::Failed);
    assert!(job.retry.next_at.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_stalled_control_or_data_connections_releases_workers_promptly() {
    for greeting in [true, false] {
        let server = Server::new(Mode::Plain).await;
        server.update(|c| {
            c.stall_greeting = greeting;
            c.stall_data = !greeting;
        });
        let (_home, store) = setup();
        let job = enqueue(&store, &server);
        let id = job.id.clone();
        let token = CancellationToken::new();
        let worker = tokio::spawn(engine::run(
            store.clone(),
            job,
            token.clone(),
            Limiter::default(),
        ));
        until(|| {
            if greeting {
                server.state.connections.load(Ordering::SeqCst) > 0
            } else {
                server.commands().iter().any(|s| s.starts_with("RETR"))
            }
        })
        .await;
        token.cancel();
        tokio::time::timeout(Duration::from_secs(2), worker)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let job = store.get(&id).unwrap().unwrap();
        assert_eq!(job.status, Status::Paused);
        assert!(job.saved_path.is_none());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn global_and_file_speed_limits_apply_and_can_be_changed_while_ftp_is_running() {
    for global in [true, false] {
        let server = Server::new(Mode::Plain).await;
        server.update(|c| c.body.truncate(8192));
        let (_home, store) = setup();
        let job = enqueue(&store, &server);
        let id = job.id.clone();
        let mut limits = Limits::default();
        if global {
            limits.global = Some(8192);
        } else {
            limits.downloads.insert(id.clone(), 8192);
        }
        let limiter = Limiter::new(&limits).unwrap();
        let begin = std::time::Instant::now();
        engine::run(store.clone(), job, CancellationToken::new(), limiter)
            .await
            .unwrap();
        assert!(begin.elapsed() >= Duration::from_millis(850));
        assert_complete(
            &store.get(&id).unwrap().unwrap(),
            &server.state.config.lock().unwrap().body,
        );
    }
    let server = Server::new(Mode::Plain).await;
    let (_home, store) = setup();
    let job = enqueue(&store, &server);
    let id = job.id.clone();
    let limiter = Limiter::new(&Limits {
        global: Some(1024),
        ..Limits::default()
    })
    .unwrap();
    let worker = tokio::spawn(engine::run(
        store.clone(),
        job,
        CancellationToken::new(),
        limiter.clone(),
    ));
    let part = store.paths.parts(&id).unwrap().join("ftp.part");
    until(|| std::fs::metadata(&part).is_ok_and(|m| m.len() > 0)).await;
    limiter.update(&Limits::default()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), worker)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_complete(
        &store.get(&id).unwrap().unwrap(),
        &server.state.config.lock().unwrap().body,
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn data_quota_preserves_ftp_partial_and_counts_prefix_verification_after_resume() {
    let server = Server::new(Mode::Plain).await;
    server.update(|c| c.body = vec![42; 2 * 1024 * 1024]);
    let (_home, store) = setup();
    store
        .set_quota_limits_at(
            download_engine::quota::Limits {
                hourly: Some(download_engine::quota::MIN_LIMIT),
                daily: None,
            },
            chrono::Utc::now().timestamp(),
        )
        .unwrap();
    let partial = run(&store, enqueue(&store, &server)).await;
    assert_eq!(partial.status, Status::Queued);
    assert!(partial.quota_waiting);
    assert!(partial.downloaded > 0);
    let count = partial.downloaded;
    store
        .set_quota_limits_at(
            download_engine::quota::Limits {
                hourly: Some(8 * download_engine::quota::MIN_LIMIT),
                daily: None,
            },
            chrono::Utc::now().timestamp(),
        )
        .unwrap();
    let complete = run(&store, partial).await;
    assert_complete(&complete, &server.state.config.lock().unwrap().body);
    assert!(store.quota_status().unwrap().hourly_used >= count + complete.downloaded);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn queue_start_pause_restart_and_parallel_limit_preserve_ftp_progress() {
    let server = Server::new(Mode::Plain).await;
    server.update(|c| c.delay = Duration::from_millis(20));
    let (_home, store) = setup();
    let now = chrono::Utc::now().timestamp();
    store.schedule_queue("main", now + 3600, now).unwrap();
    store.set_queue_limit("main", 1).unwrap();
    let first = enqueue(&store, &server);
    let second = enqueue(&store, &server);
    let scheduler = Scheduler::start(store.clone()).unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(server.state.connections.load(Ordering::SeqCst), 0);
    scheduler.send(Command::StartQueue("main".into())).unwrap();
    let part = store.paths.parts(&first.id).unwrap().join("ftp.part");
    until(|| std::fs::metadata(&part).is_ok_and(|m| m.len() >= 16384)).await;
    assert_eq!(server.state.connections.load(Ordering::SeqCst), 1);
    scheduler.send(Command::PauseQueue("main".into())).unwrap();
    until(|| {
        store
            .get(&first.id)
            .unwrap()
            .is_some_and(|j| j.status == Status::Queued && j.downloaded > 0)
            && !store.queues().unwrap()[0].running
    })
    .await;
    assert_eq!(store.get(&second.id).unwrap().unwrap().downloaded, 0);
    // Individual Restart discards the FTP identity and part before requeueing.
    scheduler.send(Command::Pause(first.id.clone())).unwrap();
    until(|| store.get(&first.id).unwrap().unwrap().status == Status::Paused).await;
    server.update(|c| {
        c.delay = Duration::ZERO;
        c.body[0] ^= 255;
        c.modified = "20261013120000.000".into();
    });
    scheduler.send(Command::Restart(first.id.clone())).unwrap();
    until(|| store.get(&first.id).unwrap().unwrap().downloaded == 0).await;
    scheduler.send(Command::StartQueue("main".into())).unwrap();
    until(|| {
        store
            .list()
            .unwrap()
            .iter()
            .all(|j| j.status == Status::Complete)
    })
    .await;
    for job in store.list().unwrap() {
        assert_complete(&job, &server.state.config.lock().unwrap().body);
    }
    assert_ne!(
        store.get(&first.id).unwrap().unwrap().saved_path,
        store.get(&second.id).unwrap().unwrap().saved_path
    );
    assert!(scheduler.errors.try_iter().collect::<Vec<_>>().is_empty());
    drop(scheduler);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scheduled_retry_releases_queue_slot_and_runs_automatically_after_delay() {
    let unavailable = Server::new(Mode::Plain).await;
    unavailable.update(|c| c.retr_status = Some(450));
    let available = Server::new(Mode::Plain).await;
    let (_home, store) = setup();
    store.set_queue_limit("main", 1).unwrap();
    let first = enqueue(&store, &unavailable);
    let second = enqueue(&store, &available);
    let scheduler = Scheduler::start(store.clone()).unwrap();
    until(|| {
        store
            .get(&first.id)
            .unwrap()
            .unwrap()
            .retry
            .next_at
            .is_some()
    })
    .await;
    until(|| store.get(&second.id).unwrap().unwrap().status == Status::Complete).await;
    unavailable.update(|c| c.retr_status = None);
    until(|| store.get(&first.id).unwrap().unwrap().status == Status::Complete).await;
    assert!(unavailable.state.connections.load(Ordering::SeqCst) >= 2);
    assert!(scheduler.errors.try_iter().collect::<Vec<_>>().is_empty());
    drop(scheduler);
}

#[test]
fn desktop_batches_accept_ftp_but_browser_messages_and_sync_checks_keep_their_http_boundary() {
    let (_home, store) = setup();
    let batch = store
        .new_batch_downloads(
            "ftp://example.test/a.bin\nftps://example.test:21/b.bin",
            None,
        )
        .unwrap();
    assert_eq!(batch.len(), 2);
    store.enqueue_batch(&batch, "main").unwrap();
    for request in [
        serde_json::json!({"action":"add","request_id":"fixture","url":"ftp://example.test/c.bin"}),
        serde_json::json!({"action":"capture_prepare","request_id":format!("capture:{}",uuid::Uuid::new_v4()),"url":"ftp://example.test/d.bin","size":12,"mime":"application/octet-stream"}),
        serde_json::json!({"action":"add_batch","request_id":format!("batch:{}",uuid::Uuid::new_v4()),"links":[{"url":"ftp://example.test/e.bin"}],"queue_id":"main","destination":"automatic"}),
    ] {
        let reply = download_engine::native::handle(&store, &serde_json::to_vec(&request).unwrap());
        assert!(!reply.ok);
        assert!(reply.message.contains("HTTP"), "{}", reply.message);
    }
    assert_eq!(store.list().unwrap().len(), 2);
    let queue = store.create_sync_queue("HTTP updates").unwrap();
    let mut job = batch[0].clone();
    job.status = Status::Complete;
    job.downloaded = 5;
    job.total = Some(5);
    job.saved_path = Some(store.paths.downloads.join("previous.bin"));
    store.save(&job).unwrap();
    store.move_job(&job.id, &queue.id).unwrap();
    store.set_queue_running(&queue.id, true).unwrap();
    assert_eq!(
        store.get(&job.id).unwrap().unwrap().status,
        Status::Complete
    );
    let mut conflicting = batch[1].clone();
    conflicting.ftp.as_mut().unwrap().security = download_engine::ftp::Security::Implicit;
    assert!(store.enqueue(&conflicting).is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ftp_paths_use_directory_commands_and_decode_components_without_command_injection() {
    let server = Server::new(Mode::Plain).await;
    let (_home, store) = setup();
    let url = server.url(false).replace(
        "fixture.bin",
        "%2Freleases/archive%20files/report%3Bfinal.bin",
    );
    let job = Job::new(
        &url,
        None,
        store.paths.downloads.clone(),
        "path-test".into(),
    )
    .unwrap();
    store.enqueue(&job).unwrap();
    let complete = run(&store, job).await;
    assert_complete(&complete, &server.state.config.lock().unwrap().body);
    let commands = server.commands();
    assert!(commands.contains(&"CWD /releases".into()));
    assert!(commands.contains(&"CWD archive files".into()));
    assert!(commands.contains(&"SIZE report;final.bin".into()));
    assert!(commands.contains(&"RETR report;final.bin".into()));
    for suffix in [
        "bad%0d%0aPASS%20secret.bin",
        "bad%00.bin",
        "file.bin;type=a",
        "a\\b.bin",
        "dir/",
    ] {
        assert!(
            Job::new(
                &server.url(false).replace("fixture.bin", suffix),
                None,
                store.paths.downloads.clone(),
                "invalid".into()
            )
            .is_err()
        );
    }
    assert_eq!(server.state.connections.load(Ordering::SeqCst), 1);
}
