use download_engine::{
    bandwidth::{Limiter, Limits, MAX_RATE, MIN_RATE, Update},
    model::{Job, Status},
    paths::AppPaths,
    store::Store,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

async fn consume(limiter: &Limiter, job: &str, mut bytes: usize) {
    let cancel = CancellationToken::new();
    while bytes > 0 {
        bytes -= limiter.acquire(job, bytes, &cancel).await.unwrap();
    }
}
#[tokio::test(start_paused = true)]
async fn aggregate_cap_is_shared_by_files_and_all_eight_connections() {
    let global = 128 * 1024;
    let per_file = 32 * 1024;
    let limiter = Limiter::new(&Limits {
        global: Some(global),
        downloads: HashMap::from([("a".into(), per_file)]),
    })
    .unwrap();
    let started = Instant::now();
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut tasks = tokio::task::JoinSet::new();
    for job in ["a", "a", "a", "a", "b", "b", "b", "b"] {
        let limiter = limiter.clone();
        let log = log.clone();
        tasks.spawn(async move {
            let cancel = CancellationToken::new();
            let mut remaining = 32 * 1024;
            while remaining > 0 {
                let n = limiter.acquire(job, remaining, &cancel).await.unwrap();
                log.lock().unwrap().push((Instant::now(), job, n));
                remaining -= n;
            }
        });
    }
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }
    let mut total = 0;
    let mut file = 0;
    let mut entries = log.lock().unwrap().clone();
    entries.sort_by_key(|e| e.0);
    for (at, job, n) in entries {
        total += n;
        if job == "a" {
            file += n;
        }
        let seconds = at.duration_since(started).as_secs_f64();
        assert!(
            total as f64 <= seconds * global as f64 + 1.0,
            "Global cap exceeded at {seconds}s: {total}"
        );
        assert!(
            file as f64 <= seconds * per_file as f64 + 1.0,
            "File cap exceeded at {seconds}s: {file}"
        );
    }
    assert_eq!(total, 256 * 1024);
    assert_eq!(file, 128 * 1024);
    assert!(
        started.elapsed() < Duration::from_secs(6),
        "Waiters starved"
    );
}
#[tokio::test(start_paused = true)]
async fn slow_file_does_not_reserve_global_capacity_or_block_other_files() {
    let limiter = Limiter::new(&Limits {
        global: Some(1024 * 1024),
        downloads: HashMap::from([("slow".into(), 1024)]),
    })
    .unwrap();
    let other = limiter.clone();
    let slow = tokio::spawn(async move {
        consume(&other, "slow", 4096).await;
    });
    tokio::task::yield_now().await;
    let start = Instant::now();
    consume(&limiter, "fast", 256 * 1024).await;
    assert!(start.elapsed() < Duration::from_secs(1));
    assert!(!slow.is_finished());
    slow.await.unwrap();
}
#[tokio::test(start_paused = true)]
async fn changing_caps_wakes_waiters_and_lowering_cap_discards_old_credit() {
    let limiter = Limiter::new(&Limits {
        global: Some(1024),
        ..Default::default()
    })
    .unwrap();
    let other = limiter.clone();
    let task = tokio::spawn(async move {
        consume(&other, "a", 64 * 1024).await;
    });
    tokio::task::yield_now().await;
    let before = Instant::now();
    limiter.update(&Limits::default()).unwrap();
    task.await.unwrap();
    assert!(before.elapsed() < Duration::from_millis(10));
    limiter
        .update(&Limits {
            global: Some(MAX_RATE),
            ..Default::default()
        })
        .unwrap();
    tokio::time::advance(Duration::from_secs(1)).await;
    limiter
        .update(&Limits {
            global: Some(1024),
            ..Default::default()
        })
        .unwrap();
    let before = Instant::now();
    consume(&limiter, "a", 2048).await;
    assert!(before.elapsed() >= Duration::from_secs(2));
}
#[tokio::test(start_paused = true)]
async fn cancellation_and_dropping_a_waiter_release_its_place() {
    let limiter = Limiter::new(&Limits {
        global: Some(1024),
        ..Default::default()
    })
    .unwrap();
    let token = CancellationToken::new();
    let other = limiter.clone();
    let child = token.clone();
    let task = tokio::spawn(async move { other.acquire("a", 8192, &child).await });
    tokio::task::yield_now().await;
    token.cancel();
    let before = Instant::now();
    assert_eq!(task.await.unwrap(), None);
    assert_eq!(before.elapsed(), Duration::ZERO);
    let other = limiter.clone();
    let abandoned = tokio::spawn(async move {
        consume(&other, "b", 8192).await;
    });
    tokio::task::yield_now().await;
    abandoned.abort();
    assert!(abandoned.await.unwrap_err().is_cancelled());
    consume(&limiter, "c", 1024).await;
    assert!(before.elapsed() < Duration::from_secs(2));
}
#[tokio::test(start_paused = true)]
async fn unrelated_updates_preserve_global_credit_and_unlimited_is_immediate() {
    let limiter = Limiter::new(&Limits {
        global: Some(1024),
        ..Default::default()
    })
    .unwrap();
    tokio::time::advance(Duration::from_millis(50)).await;
    limiter
        .update(&Limits {
            global: Some(1024),
            downloads: HashMap::from([("other".into(), 1024)]),
        })
        .unwrap();
    let start = Instant::now();
    consume(&limiter, "a", 50).await;
    assert_eq!(start.elapsed(), Duration::ZERO);
    limiter.update(&Limits::default()).unwrap();
    consume(&limiter, "a", 1024 * 1024).await;
    assert_eq!(start.elapsed(), Duration::ZERO);
}
#[test]
fn persistence_is_atomic_survives_worker_saves_and_restart_and_cascades_on_remove() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    };
    let store = Store::open(paths.clone()).unwrap();
    assert_eq!(store.bandwidth_limits().unwrap(), Limits::default());
    let mut job = Job::new(
        "https://example.com/a",
        None,
        paths.downloads.clone(),
        Uuid::new_v4().to_string(),
    )
    .unwrap();
    store.enqueue(&job).unwrap();
    let mut update = Update {
        request: Uuid::new_v4(),
        global: Some(4096),
        download: Some((job.id.clone(), Some(2048))),
    };
    let saved = store.set_bandwidth_limits(&update).unwrap();
    job.downloaded = 123;
    job.status = Status::Paused;
    job.retry.attempts = 2;
    store.save(&job).unwrap();
    assert_eq!(store.bandwidth_limits().unwrap(), saved);
    for bad in [0, MIN_RATE - 1, MAX_RATE + 1, u64::MAX] {
        update.global = Some(bad);
        assert!(store.set_bandwidth_limits(&update).is_err());
        assert_eq!(store.bandwidth_limits().unwrap(), saved);
        assert!(
            Limiter::new(&Limits {
                global: Some(bad),
                ..Default::default()
            })
            .is_err()
        );
    }
    update.global = None;
    update.download = Some(("removed".into(), Some(1024)));
    assert!(store.set_bandwidth_limits(&update).is_err());
    assert_eq!(store.bandwidth_limits().unwrap(), saved);
    drop(store);
    let store = Store::open(paths).unwrap();
    store.recover().unwrap();
    assert_eq!(store.bandwidth_limits().unwrap(), saved);
    let recovered = store.get(&job.id).unwrap().unwrap();
    assert_eq!(recovered.retry.attempts, 2);
    assert_eq!(recovered.downloaded, 123);
    update.download = None;
    store.set_bandwidth_limits(&update).unwrap();
    assert_eq!(
        store.bandwidth_limits().unwrap().downloads.get(&job.id),
        Some(&2048)
    );
    store.remove(&job.id).unwrap();
    assert_eq!(store.bandwidth_limits().unwrap(), Limits::default());
}

#[tokio::test(start_paused = true)]
async fn late_timer_wake_preserves_credit_instead_of_reducing_throughput() {
    let limiter = Limiter::new(&Limits {
        global: Some(4 * 1024 * 1024),
        ..Default::default()
    })
    .unwrap();
    tokio::time::advance(Duration::from_millis(40)).await;
    let before = Instant::now();
    consume(&limiter, "a", 128 * 1024).await;
    assert_eq!(before.elapsed(), Duration::ZERO);
}

#[tokio::test(start_paused = true)]
async fn newly_eligible_owner_is_woken_after_another_waiter_dropped_its_timer() {
    use std::{
        future::Future,
        sync::atomic::{AtomicUsize, Ordering},
        task::{Context, Poll, Wake, Waker},
    };
    #[derive(Default)]
    struct WakeCount(AtomicUsize);
    impl Wake for WakeCount {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let limiter = Limiter::new(&Limits {
        global: Some(1024),
        downloads: HashMap::from([("a".into(), 1024)]),
    })
    .unwrap();
    let cancel = CancellationToken::new();
    let first_wake = Arc::new(WakeCount::default());
    let second_wake = Arc::new(WakeCount::default());
    let first_waker = Waker::from(first_wake.clone());
    let second_waker = Waker::from(second_wake.clone());
    let mut a = Box::pin(limiter.acquire("a", 51, &cancel));
    let mut b = Box::pin(limiter.acquire("b", 1, &cancel));
    assert!(
        a.as_mut()
            .poll(&mut Context::from_waker(&first_waker))
            .is_pending()
    );
    assert!(
        b.as_mut()
            .poll(&mut Context::from_waker(&second_waker))
            .is_pending()
    );
    tokio::time::advance(Duration::from_millis(1)).await;
    // B can now take its small grant. A waits for B and releases A's timer.
    assert!(
        a.as_mut()
            .poll(&mut Context::from_waker(&first_waker))
            .is_pending()
    );
    tokio::time::advance(Duration::from_millis(50)).await;
    let wakes_before = first_wake.0.load(Ordering::SeqCst);
    // Before B is polled, A regains eligibility and is first in line. B must
    // wake A explicitly: neither task can assume A still has a timer armed.
    assert!(
        b.as_mut()
            .poll(&mut Context::from_waker(&second_waker))
            .is_pending()
    );
    assert!(
        first_wake.0.load(Ordering::SeqCst) > wakes_before,
        "The newly eligible owner was left asleep with no timer"
    );
    assert_eq!(
        a.as_mut().poll(&mut Context::from_waker(&first_waker)),
        Poll::Ready(Some(51))
    );
    assert_eq!(
        b.as_mut().poll(&mut Context::from_waker(&second_waker)),
        Poll::Ready(Some(1))
    );
}
