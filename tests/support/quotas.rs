use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quota_is_shared_by_simultaneous_files_in_different_queues() {
    use download_engine::quota::{Limits, MIN_LIMIT};
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let fixture = Fixture::with_body_gate(8 * MIN_LIMIT as usize, 0, true, Some(gate)).await;
    let (_temp, store) = setup();
    store
        .set_quota_limits_at(
            Limits {
                hourly: Some(MIN_LIMIT),
                daily: None,
            },
            chrono::Utc::now().timestamp(),
        )
        .unwrap();
    let queue = store.create_queue("Another queue").unwrap();
    store.set_queue_running(&queue.id, true).unwrap();
    let mut ids = Vec::new();
    for q in ["main", queue.id.as_str()] {
        let mut job = enqueue(&store, &fixture.url);
        job.status = Status::Queued;
        store.save(&job).unwrap();
        store.move_job(&job.id, q).unwrap();
        ids.push(job.id);
    }
    let scheduler = Scheduler::start(store.clone()).unwrap();
    until_stalled(&store, async {
        // Each file probes and opens four ranges before any payload is released.
        while fixture.requests.lock().unwrap().len() < 10 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        fixture.release_body();
        loop {
            let jobs = store.list().unwrap();
            if jobs
                .iter()
                .all(|j| j.quota_waiting && j.status == Status::Queued)
            {
                assert!(
                    jobs.iter()
                        .all(|j| j.downloaded > 0 && j.downloaded < fixture.data.len() as u64)
                );
                let usage = store.quota_status().unwrap();
                assert!(usage.blocked());
                assert!(usage.hourly_used >= jobs.iter().map(|j| j.downloaded).sum::<u64>());
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "Quota hold stalled; requests={}; jobs={:?}",
            fixture.requests.lock().unwrap().len(),
            store.list()
        )
    });
    assert_eq!(store.list().unwrap().len(), ids.len());
    assert!(scheduler.errors.try_iter().collect::<Vec<_>>().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quota_database_write_failure_stops_transfer_without_retrying_or_publishing() {
    use download_engine::quota::{Limits, MIN_LIMIT};
    let fixture = Fixture::new(MIN_LIMIT as usize, 0, true).await;
    let (_temp, store) = setup();
    store
        .set_quota_limits_at(
            Limits {
                hourly: Some(MIN_LIMIT),
                daily: None,
            },
            chrono::Utc::now().timestamp(),
        )
        .unwrap();
    let conn = rusqlite::Connection::open(store.paths.data.join("queue.sqlite3")).unwrap();
    conn.execute_batch("CREATE TRIGGER reject_payload BEFORE UPDATE ON data_quotas WHEN NEW.hourly_used>OLD.hourly_used BEGIN SELECT RAISE(ABORT,'fixture quota write failed'); END;").unwrap();
    let stopped = run(&store, enqueue(&store, &fixture.url)).await;
    assert_eq!(stopped.status, Status::Failed);
    assert!(
        stopped
            .error
            .unwrap()
            .contains("fixture quota write failed")
    );
    assert_eq!(stopped.downloaded, 0);
    assert_eq!(stopped.retry.attempts, 0);
    assert!(stopped.saved_path.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quota_preserves_range_bytes_across_restart_and_resumes_exactly_after_reset() {
    use download_engine::quota::{HOUR, Limits, MIN_LIMIT};
    let fixture = Fixture::new(4 * MIN_LIMIT as usize, 0, true).await;
    let (_temp, store) = setup();
    let now = chrono::Utc::now().timestamp();
    store
        .set_quota_limits_at(
            Limits {
                hourly: Some(MIN_LIMIT),
                daily: None,
            },
            now,
        )
        .unwrap();
    let mut job = enqueue(&store, &fixture.url);
    job.retry.attempts = 2;
    let held = run(&store, job).await;
    assert_eq!(held.status, Status::Queued, "{:?}", held.error);
    assert!(held.quota_waiting);
    assert!(held.downloaded >= MIN_LIMIT && held.downloaded < fixture.data.len() as u64);
    assert_eq!(held.retry.attempts, 2);
    assert!(held.retry.next_at.is_none());
    let usage = store.quota_status().unwrap();
    assert!(usage.hourly_used >= held.downloaded);
    let requests = fixture.requests.lock().unwrap().len();
    let reopened = Store::open(store.paths.clone()).unwrap();
    let again = run(&reopened, held.clone()).await;
    assert_eq!(again.downloaded, held.downloaded);
    assert!(again.quota_waiting);
    assert_eq!(fixture.requests.lock().unwrap().len(), requests);
    // Reset the hour and raise its allowance so the remaining bytes fit.
    reopened.quota_status_at(now + HOUR).unwrap();
    reopened
        .set_quota_limits_at(
            Limits {
                hourly: Some(8 * MIN_LIMIT),
                daily: None,
            },
            now + HOUR,
        )
        .unwrap();
    let done = run(&reopened, again).await;
    assert_eq!(done.status, Status::Complete, "{:?}", done.error);
    assert!(!done.quota_waiting);
    assert_eq!(
        std::fs::read(done.saved_path.unwrap()).unwrap(),
        *fixture.data
    );
    let final_usage = reopened.quota_status().unwrap();
    assert_eq!(
        final_usage.hourly_used,
        fixture.data.len() as u64 - held.downloaded
    );
    assert_eq!(
        final_usage.daily_used,
        usage.daily_used + final_usage.hourly_used
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quota_nonresumable_file_requires_manual_action_and_exact_boundary_can_finish() {
    use download_engine::quota::{Limits, MIN_LIMIT};
    let fixture = Fixture::new(3 * MIN_LIMIT as usize, 1, true).await;
    let (_temp, store) = setup();
    store
        .set_quota_limits_at(
            Limits {
                hourly: Some(MIN_LIMIT),
                daily: None,
            },
            chrono::Utc::now().timestamp(),
        )
        .unwrap();
    let held = run(&store, enqueue(&store, &fixture.url)).await;
    assert_eq!(held.status, Status::Paused);
    assert!(held.quota_waiting);
    assert!(held.error.unwrap().contains("cannot safely resume"));
    assert_eq!(held.retry.attempts, 0);
    let small = Fixture::new(MIN_LIMIT as usize, 1, true).await;
    let (_other, fresh) = setup();
    fresh
        .set_quota_limits_at(
            Limits {
                hourly: Some(MIN_LIMIT),
                daily: None,
            },
            chrono::Utc::now().timestamp(),
        )
        .unwrap();
    let complete = run(&fresh, enqueue(&fresh, &small.url)).await;
    assert_eq!(complete.status, Status::Complete, "{:?}", complete.error);
    assert_eq!(fresh.quota_status().unwrap().hourly_used, MIN_LIMIT);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quota_failed_attempt_payload_is_counted_again_on_retry() {
    use download_engine::quota::{Limits, MIN_LIMIT};
    let fixture = Fixture::new(1024 * 1024, 4, true).await;
    let (_temp, store) = setup();
    store
        .set_quota_limits_at(
            Limits {
                hourly: Some(8 * MIN_LIMIT),
                daily: None,
            },
            chrono::Utc::now().timestamp(),
        )
        .unwrap();
    let first = run(&store, enqueue(&store, &fixture.url)).await;
    assert_eq!(first.status, Status::Queued);
    let before = store.quota_status().unwrap().daily_used;
    assert!(before > 0);
    fixture.mode.store(0, Ordering::SeqCst);
    let done = run(&store, first.clone()).await;
    assert_eq!(done.status, Status::Complete);
    assert_eq!(
        store.quota_status().unwrap().daily_used,
        before + fixture.data.len() as u64 - first.downloaded
    );
    assert_eq!(
        std::fs::read(done.saved_path.unwrap()).unwrap(),
        *fixture.data
    );
}

async fn quota_barrier(scheduler: &Scheduler, limits: download_engine::quota::Limits) {
    let request = uuid::Uuid::new_v4();
    scheduler
        .send(Command::Quota(download_engine::quota::Update {
            request,
            limits,
        }))
        .unwrap();
    let deadline = tokio::time::Instant::now() + PROGRESS_LIMIT;
    loop {
        if let Ok(event) = scheduler.quota_events.try_recv() {
            assert_eq!(event.request, request);
            event.result.unwrap();
            return;
        }
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quota_scheduler_restart_and_reset_respect_manual_pauses_and_stopped_queues() {
    use download_engine::quota::{HOUR, Limits, MIN_LIMIT};
    let fixture = Fixture::new(256 * 1024, 0, true).await;
    let (_temp, store) = setup();
    let now = chrono::Utc::now().timestamp();
    let limits = Limits {
        hourly: Some(MIN_LIMIT),
        daily: None,
    };
    store.set_quota_limits_at(limits, now).unwrap();
    store.record_quota_bytes_at(MIN_LIMIT, now).unwrap();
    let mut waiting = enqueue(&store, &fixture.url);
    waiting.status = Status::Queued;
    waiting.quota_waiting = true;
    store.save(&waiting).unwrap();
    let mut paused = enqueue(&store, &fixture.url);
    paused.status = Status::Queued;
    paused.quota_waiting = true;
    store.save(&paused).unwrap();
    let later = store.create_queue("Later").unwrap();
    store.set_queue_running(&later.id, false).unwrap();
    let mut stopped = enqueue(&store, &fixture.url);
    stopped.status = Status::Queued;
    store.save(&stopped).unwrap();
    store.move_job(&stopped.id, &later.id).unwrap();
    let scheduler = Scheduler::start(store.clone()).unwrap();
    scheduler.send(Command::Pause(paused.id.clone())).unwrap();
    quota_barrier(&scheduler, limits).await;
    assert_eq!(
        store.get(&paused.id).unwrap().unwrap().status,
        Status::Paused
    );
    assert!(fixture.requests.lock().unwrap().is_empty());
    drop(scheduler);
    let reopened = Store::open(store.paths.clone()).unwrap();
    let scheduler = Scheduler::start(reopened.clone()).unwrap();
    quota_barrier(&scheduler, limits).await;
    assert_eq!(
        reopened.get(&waiting.id).unwrap().unwrap().status,
        Status::Queued
    );
    assert!(fixture.requests.lock().unwrap().is_empty());
    reopened.quota_status_at(now + HOUR).unwrap();
    let done = until_stalled(&reopened, async {
        loop {
            let job = read_job(&reopened, &waiting.id).await.unwrap();
            if job.status == Status::Complete {
                break job;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("Quota reset stalled: {:?}", reopened.get(&waiting.id)));
    assert_eq!(
        std::fs::read(done.saved_path.unwrap()).unwrap(),
        *fixture.data
    );
    let manual = reopened.get(&paused.id).unwrap().unwrap();
    assert_eq!(manual.status, Status::Paused);
    assert!(!manual.quota_waiting);
    assert_eq!(
        reopened.get(&stopped.id).unwrap().unwrap().status,
        Status::Queued
    );
    assert_eq!(
        reopened.quota_status().unwrap().hourly_used,
        fixture.data.len() as u64
    );
    assert!(scheduler.errors.try_iter().collect::<Vec<_>>().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quota_link_check_counts_payload_and_does_not_change_saved_parts_when_blocked() {
    use download_engine::{
        link_recovery::{Command as LinkCommand, Event as LinkEvent},
        quota::{HOUR, Limits, MIN_LIMIT},
    };
    let fixture = Fixture::new(4 * MIN_LIMIT as usize, 0, true).await;
    let (_temp, store) = setup();
    let now = chrono::Utc::now().timestamp();
    let limits = Limits {
        hourly: Some(MIN_LIMIT),
        daily: None,
    };
    store.set_quota_limits_at(limits, now).unwrap();
    let mut held = run(&store, enqueue(&store, &fixture.url)).await;
    assert!(held.quota_waiting);
    held.status = Status::Paused;
    held.quota_waiting = false;
    store.save(&held).unwrap();
    let snapshot = || {
        let mut paths = std::fs::read_dir(store.paths.parts(&held.id).unwrap())
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect::<Vec<_>>();
        paths.sort();
        paths
            .into_iter()
            .map(|p| (p.clone(), std::fs::read(p).unwrap()))
            .collect::<Vec<_>>()
    };
    let before = snapshot();
    // Leave just 64 KiB for verification in the next hour.
    store.quota_status_at(now + HOUR).unwrap();
    store
        .record_quota_bytes_at(MIN_LIMIT - 65536, now + HOUR)
        .unwrap();
    let scheduler = Scheduler::start(store.clone()).unwrap();
    let request = uuid::Uuid::new_v4();
    scheduler
        .send(Command::Link(LinkCommand::Check {
            request,
            job: held.id.clone(),
            url: fixture.url.clone(),
        }))
        .unwrap();
    let deadline = tokio::time::Instant::now() + PROGRESS_LIMIT;
    loop {
        if let Ok(event) = scheduler.link_events.try_recv() {
            match event {
                LinkEvent::Progress { .. } => {}
                LinkEvent::Failed {
                    request: got,
                    message,
                } => {
                    assert_eq!(got, request);
                    assert!(message.contains("Data quota reached"), "{message}");
                    break;
                }
                other => panic!("Unexpected link event: {other:?}"),
            }
        }
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(store.quota_status().unwrap().blocked());
    assert_eq!(snapshot(), before);
    let after = store.get(&held.id).unwrap().unwrap();
    assert_eq!(after.status, Status::Paused);
    assert_eq!(after.downloaded, held.downloaded);
}
