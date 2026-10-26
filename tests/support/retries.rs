use super::*;
use download_engine::{queue::MAIN_QUEUE, retry::MAX_RETRIES};
use std::sync::atomic::AtomicUsize;

struct Replies {
    url: String,
    count: Arc<AtomicUsize>,
    times: Arc<Mutex<Vec<i64>>>,
    task: JoinHandle<()>,
}
impl Drop for Replies {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Replies {
    async fn new(status: u16, extra: &str, failures: usize, truncated: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/file?private-test-token",
            listener.local_addr().unwrap()
        );
        let count = Arc::new(AtomicUsize::new(0));
        let times = Arc::new(Mutex::new(Vec::new()));
        let (requests, seen) = (count.clone(), times.clone());
        let extra = extra.to_owned();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let mut raw = Vec::new();
                let mut byte = [0];
                while raw.len() < 32768 {
                    if stream.read(&mut byte).await.unwrap_or(0) == 0 {
                        break;
                    }
                    raw.push(byte[0]);
                    if raw.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                seen.lock()
                    .unwrap()
                    .push(chrono::Utc::now().timestamp_millis());
                let failed = requests.fetch_add(1, Ordering::SeqCst) < failures;
                let (code, headers, body, size) = if failed && truncated {
                    (200, "", "abc", 6)
                } else if failed {
                    (status, extra.as_str(), "", 0)
                } else {
                    (200, "", "abcdef", 6)
                };
                let response = format!(
                    "HTTP/1.1 {code} Test\r\nContent-Length: {size}\r\nConnection: close\r\n{headers}\r\n{body}"
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        Self {
            url,
            count,
            times,
            task,
        }
    }
}
fn queued(store: &Store, url: &str) -> Job {
    let job = Job::new(
        url,
        Some("fixture.bin"),
        store.paths.downloads.clone(),
        uuid::Uuid::new_v4().to_string(),
    )
    .unwrap();
    store.enqueue(&job).unwrap()
}

async fn waiting(store: &Store, id: &str) -> Job {
    wait_until(store, || {
        let job = store.get(id).unwrap().unwrap();
        assert_ne!(job.status, Status::Failed, "{:?}", job.error);
        job.retry.next_at.is_some()
    })
    .await;
    store.get(id).unwrap().unwrap()
}
async fn completed(store: &Store, id: &str) -> Job {
    until_stalled(store, async {
        loop {
            let job = store.get(id).unwrap().unwrap();
            assert_ne!(job.status, Status::Failed, "{:?}", job.error);
            if job.status == Status::Complete {
                return job;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "Download completion stalled: {}",
            describe_job(&store.get(id).unwrap().unwrap())
        )
    })
}
async fn beyond(at: i64) {
    let millis = (at - chrono::Utc::now().timestamp_millis()).max(0) as u64;
    tokio::time::sleep(Duration::from_millis(millis + 700)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn temporary_failure_releases_queue_slot_and_recovers_after_deadline() {
    let failing = Replies::new(503, "", 1, false).await;
    let healthy = Replies::new(200, "", 0, false).await;
    let (_temp, store) = setup();
    store.set_queue_limit(MAIN_QUEUE, 1).unwrap();
    let first = queued(&store, &failing.url);
    let second = queued(&store, &healthy.url);
    let scheduler = Scheduler::start(store.clone()).unwrap();
    let pending = waiting(&store, &first.id).await;
    let second = completed(&store, &second.id).await;
    assert_eq!(
        std::fs::read(second.saved_path.unwrap()).unwrap(),
        b"abcdef"
    );
    let final_job = completed(&store, &first.id).await;
    assert_eq!(failing.count.load(Ordering::SeqCst), 2);
    assert_eq!(healthy.count.load(Ordering::SeqCst), 1);
    let failed_times = failing.times.lock().unwrap().clone();
    let healthy_at = healthy.times.lock().unwrap()[0];
    // Observe request order at the servers. The assertion thread may resume
    // after the retry deadline even though the healthy job used the freed slot.
    assert!(
        failed_times[0] <= healthy_at && healthy_at < failed_times[1],
        "Healthy request {healthy_at} did not run between failed attempts {failed_times:?}"
    );
    assert!(failed_times[1] >= pending.retry.next_at.unwrap());
    assert!(final_job.retry.is_default());
    assert_eq!(
        std::fs::read(final_job.saved_path.unwrap()).unwrap(),
        b"abcdef"
    );
    assert!(scheduler.errors.try_recv().is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn user_pause_cancels_waiting_retry_and_resume_starts_fresh() {
    let fixture = Replies::new(503, "", 1, false).await;
    let (_temp, store) = setup();
    let job = queued(&store, &fixture.url);
    let scheduler = Scheduler::start(store.clone()).unwrap();
    let pending = waiting(&store, &job.id).await;
    scheduler.send(Command::Pause(job.id.clone())).unwrap();
    wait_until(&store, || {
        store.get(&job.id).unwrap().unwrap().status == Status::Paused
    })
    .await;
    beyond(pending.retry.next_at.unwrap()).await;
    assert_eq!(fixture.count.load(Ordering::SeqCst), 1);
    assert!(store.get(&job.id).unwrap().unwrap().retry.next_at.is_none());
    scheduler.send(Command::Resume(job.id.clone())).unwrap();
    assert!(completed(&store, &job.id).await.retry.is_default());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queue_pause_and_app_restart_preserve_retry_deadline_and_limit() {
    let fixture = Replies::new(429, "Retry-After: 7\r\n", 1, false).await;
    let (_temp, store) = setup();
    let job = queued(&store, &fixture.url);
    let scheduler = Scheduler::start(store.clone()).unwrap();
    let pending = waiting(&store, &job.id).await;
    scheduler
        .send(Command::PauseQueue(MAIN_QUEUE.into()))
        .unwrap();
    wait_until(&store, || !store.queues().unwrap()[0].running).await;
    // A waiting retry holds no worker, so restarting must neither hang nor wait
    // for its deadline. Shutdown still finishes the current scheduler tick, which
    // took up to 1 s of database work under load, so bound it by the deadline.
    let retry_at = pending.retry.next_at.unwrap();
    let (stopped, shutdown) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        drop(scheduler);
        let _ = stopped.send(chrono::Utc::now().timestamp_millis());
    });
    let remaining = (retry_at - chrono::Utc::now().timestamp_millis()).max(0) as u64;
    let stopped_at = shutdown
        .recv_timeout(Duration::from_millis(remaining))
        .expect("Scheduler shutdown hung or waited for the pending retry");
    assert!(
        stopped_at < retry_at,
        "Shutdown waited for the pending retry"
    );
    let reopened = Store::open(store.paths.clone()).unwrap();
    let scheduler = Scheduler::start(reopened.clone()).unwrap();
    assert_eq!(reopened.get(&job.id).unwrap().unwrap().retry, pending.retry);
    beyond(pending.retry.next_at.unwrap()).await;
    assert_eq!(fixture.count.load(Ordering::SeqCst), 1);
    scheduler
        .send(Command::StartQueue(MAIN_QUEUE.into()))
        .unwrap();
    assert!(completed(&reopened, &job.id).await.retry.is_default());
    assert!(fixture.times.lock().unwrap()[1] >= pending.retry.next_at.unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_failures_stop_after_three_retries_even_after_reopen() {
    let fixture = Replies::new(503, "", usize::MAX, false).await;
    let (_temp, store) = setup();
    let job = queued(&store, &fixture.url);
    let scheduler = Scheduler::start(store.clone()).unwrap();
    let pending = waiting(&store, &job.id).await;
    drop(scheduler);
    let scheduler = Scheduler::start(store.clone()).unwrap();
    assert_eq!(store.get(&job.id).unwrap().unwrap().retry, pending.retry);
    // Each attempt restarts the wait, so the attempt count bounds it instead.
    until_stalled(&store, async {
        loop {
            assert!(fixture.count.load(Ordering::SeqCst) <= 1 + usize::from(MAX_RETRIES));
            if store.get(&job.id).unwrap().unwrap().status == Status::Failed {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    let failed = store.get(&job.id).unwrap().unwrap();
    assert_eq!(
        fixture.count.load(Ordering::SeqCst),
        1 + usize::from(MAX_RETRIES)
    );
    assert_eq!(failed.retry.attempts, MAX_RETRIES);
    assert!(failed.retry.next_at.is_none());
    assert!(failed.error.unwrap().contains("exhausted"));
    let times = fixture.times.lock().unwrap();
    for (pair, minimum) in times.windows(2).zip([5000, 10000, 20000]) {
        assert!(pair[1] - pair[0] >= minimum);
    }
    assert!(scheduler.errors.try_recv().is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn segmented_retry_resumes_verified_bytes_and_rejects_changed_files() {
    for changed in [false, true] {
        let fixture = Fixture::new(300000, 4, false).await;
        let (_temp, store) = setup();
        let job = queued(&store, &fixture.url);
        let scheduler = Scheduler::start(store.clone()).unwrap();
        let pending = waiting(&store, &job.id).await;
        assert!(pending.downloaded > 0);
        fixture
            .mode
            .store(if changed { 2 } else { 0 }, Ordering::SeqCst);
        if changed {
            wait_until(&store, || {
                store.get(&job.id).unwrap().unwrap().status == Status::Failed
            })
            .await;
            let failed = store.get(&job.id).unwrap().unwrap();
            assert!(failed.error.unwrap().contains("remote file changed"));
            assert!(failed.retry.next_at.is_none());
            assert!(failed.saved_path.is_none());
        } else {
            let done = completed(&store, &job.id).await;
            assert_eq!(
                std::fs::read(done.saved_path.unwrap()).unwrap(),
                *fixture.data
            );
            let expected = format!("range: bytes={}-", pending.downloaded);
            assert!(
                fixture
                    .requests
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|r| r.to_ascii_lowercase().contains(&expected))
            );
        }
        assert!(scheduler.errors.try_recv().is_err());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nonresumable_retry_discards_partial_body_and_starts_from_zero() {
    let fixture = Replies::new(200, "", 1, true).await;
    let (_temp, store) = setup();
    let job = queued(&store, &fixture.url);
    let _scheduler = Scheduler::start(store.clone()).unwrap();
    let pending = waiting(&store, &job.id).await;
    assert_eq!(pending.downloaded, 3);
    assert!(pending.plan.is_none());
    let done = completed(&store, &job.id).await;
    assert_eq!(done.downloaded, 6);
    assert_eq!(std::fs::read(done.saved_path.unwrap()).unwrap(), b"abcdef");
}

#[tokio::test]
async fn http_classification_and_retry_after_apply_to_actual_responses() {
    for status in [408, 429, 500, 502, 503, 504, 401, 403, 404, 410, 501, 507] {
        let fixture = Replies::new(status, "Retry-After: 120\r\n", usize::MAX, false).await;
        let (_temp, store) = setup();
        let before = chrono::Utc::now().timestamp_millis();
        let job = run(&store, queued(&store, &fixture.url)).await;
        let transient = matches!(status, 408 | 429 | 500 | 502 | 503 | 504);
        assert_eq!(
            job.status,
            if transient {
                Status::Queued
            } else {
                Status::Failed
            },
            "{status}"
        );
        if transient {
            assert!(job.retry.next_at.unwrap() >= before + 120000);
            assert_eq!(job.retry.attempts, 1);
        } else {
            assert!(job.retry.is_default());
        }
        assert!(!job.error.unwrap().contains("private-test-token"));
        assert_eq!(fixture.count.load(Ordering::SeqCst), 1);
    }
    for header in [
        "Retry-After: 999999999999999999999999\r\n",
        "Retry-After: 86401\r\n",
    ] {
        let fixture = Replies::new(503, header, usize::MAX, false).await;
        let (_temp, store) = setup();
        let job = run(&store, queued(&store, &fixture.url)).await;
        assert_eq!(job.status, Status::Failed);
        assert!(job.retry.is_default());
        assert!(job.error.unwrap().contains("24 hours"));
    }
}

#[tokio::test]
async fn connection_refusal_retries_but_local_disk_failure_does_not() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/file?private-test-token",
        listener.local_addr().unwrap()
    );
    drop(listener);
    let (_temp, store) = setup();
    let job = run(&store, queued(&store, &url)).await;
    assert_eq!(job.status, Status::Queued, "{:?}", job.error);
    assert_eq!(job.retry.attempts, 1);
    assert!(!job.error.unwrap().contains("private-test-token"));
    let fixture = Replies::new(200, "", 0, false).await;
    let mut job = queued(&store, &fixture.url);
    job.directory = store.paths.downloads.join("not-a-folder");
    std::fs::write(&job.directory, b"preserve").unwrap();
    let job = run(&store, job).await;
    assert_eq!(job.status, Status::Failed);
    assert!(job.retry.is_default());
    assert_eq!(fixture.count.load(Ordering::SeqCst), 0);
    assert_eq!(std::fs::read(job.directory).unwrap(), b"preserve");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn range_http_backoff_counts_one_retry_for_all_segments() {
    let fixture = Fixture::new(8 * 1024 * 1024, 15, false).await;
    let (_temp, store) = setup();
    let job = queued(&store, &fixture.url);
    let before = chrono::Utc::now().timestamp_millis();
    let _scheduler = Scheduler::start(store.clone()).unwrap();
    let pending = waiting(&store, &job.id).await;
    assert_eq!(pending.plan.as_ref().unwrap().segments.len(), 4);
    assert_eq!(pending.retry.attempts, 1);
    assert!(pending.retry.next_at.unwrap() >= before + 7000);
    assert_eq!(pending.downloaded, 0);
    assert!(pending.saved_path.is_none());
    fixture.mode.store(0, Ordering::SeqCst);
    // The server mandates seven seconds of backoff. Give the subsequent
    // transfer its normal completion budget, including under parallel disk I/O.
    beyond(pending.retry.next_at.unwrap()).await;
    let done = completed(&store, &job.id).await;
    assert_eq!(
        std::fs::read(done.saved_path.unwrap()).unwrap(),
        *fixture.data
    );
    assert!(done.retry.is_default());
}
