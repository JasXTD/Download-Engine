use super::*;
use download_engine::bandwidth::{Limits, Update};
use std::time::Instant;
use uuid::Uuid;

async fn change(
    scheduler: &Scheduler,
    global: Option<u64>,
    download: Option<(String, Option<u64>)>,
) -> Result<Limits, String> {
    let request = Uuid::new_v4();
    scheduler
        .send(Command::Bandwidth(Update {
            request,
            global,
            download,
        }))
        .unwrap();
    tokio::time::timeout(PROGRESS_LIMIT, async {
        loop {
            if let Ok(event) = scheduler.bandwidth_events.try_recv() {
                assert_eq!(event.request, request);
                return event.result;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("No speed-limit acknowledgement")
}
/// Bytes the worker has written to this job's partial files. Sizes come from
/// each file: Windows directory listings lag behind files still open for writing.
fn written(store: &Store, job: &Job) -> u64 {
    std::fs::read_dir(store.paths.parts(&job.id).unwrap())
        .unwrap()
        .map(|entry| std::fs::metadata(entry.unwrap().path()).unwrap().len())
        .sum()
}
#[tokio::test]
async fn single_stream_obeys_file_cap_and_preserves_exact_bytes() {
    let fixture = Fixture::new(128 * 1024, 1, false).await;
    let (_temp, store) = setup();
    let job = enqueue(&store, &fixture.url);
    store
        .set_bandwidth_limits(&Update {
            request: Uuid::new_v4(),
            global: None,
            download: Some((job.id.clone(), Some(64 * 1024))),
        })
        .unwrap();
    let start = Instant::now();
    let saved = run(&store, job).await;
    assert!(start.elapsed() >= Duration::from_millis(1950));
    assert_eq!(saved.status, Status::Complete, "{saved:?}");
    assert!(saved.plan.is_none());
    assert_eq!(
        std::fs::read(saved.saved_path.unwrap()).unwrap(),
        *fixture.data
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scheduler_shares_global_budget_across_queues_and_all_range_workers() {
    let fixture = Fixture::new(8 * 1024 * 1024, 0, false).await;
    let (_temp, store) = setup();
    let later = store.create_queue("Second").unwrap();
    store.set_queue_running(&later.id, true).unwrap();
    let mut jobs = Vec::new();
    for queue in ["main", &later.id] {
        let job = Job::new(
            &fixture.url,
            None,
            store.paths.downloads.clone(),
            Uuid::new_v4().to_string(),
        )
        .unwrap();
        store
            .enqueue_batch(std::slice::from_ref(&job), queue)
            .unwrap();
        jobs.push(job);
    }
    // The total budget must be the bottleneck: individual limits alone would
    // finish both files sooner and must not make this shared-cap test pass.
    let global = 4 * 1024 * 1024;
    store
        .set_bandwidth_limits(&Update {
            request: Uuid::new_v4(),
            global: Some(global),
            download: Some((jobs[0].id.clone(), Some(3 * 1024 * 1024))),
        })
        .unwrap();
    let start = Instant::now();
    let scheduler = Scheduler::start(store.clone()).unwrap();
    let result = until_stalled(&store, async {
        loop {
            if jobs
                .iter()
                .all(|j| store.get(&j.id).unwrap().unwrap().status == Status::Complete)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await;
    let states: Vec<_> = jobs
        .iter()
        .map(|j| store.get(&j.id).unwrap().unwrap())
        .collect();
    assert!(
        result.is_ok(),
        "Capped transfers did not finish: {states:?}; errors: {:?}",
        scheduler.errors.try_iter().collect::<Vec<_>>()
    );
    eprintln!(
        "Two 8 MiB files, 4 MiB/s shared cap: {:.3}s",
        start.elapsed().as_secs_f64()
    );
    assert!(start.elapsed() >= Duration::from_millis(3950));
    for job in jobs {
        let saved = store.get(&job.id).unwrap().unwrap();
        assert_eq!(saved.plan.unwrap().segments.len(), 4);
        assert_eq!(
            std::fs::read(saved.saved_path.unwrap()).unwrap(),
            *fixture.data
        );
    }
    assert!(scheduler.errors.try_recv().is_err());
}
#[tokio::test]
async fn active_updates_pause_restart_and_removal_keep_limits_and_queue_state_correct() {
    let fixture = Fixture::new(128 * 1024, 0, false).await;
    let (_temp, store) = setup();
    let job = Job::new(
        &fixture.url,
        None,
        store.paths.downloads.clone(),
        Uuid::new_v4().to_string(),
    )
    .unwrap();
    store.enqueue(&job).unwrap();
    store
        .set_bandwidth_limits(&Update {
            request: Uuid::new_v4(),
            global: Some(1024),
            download: Some((job.id.clone(), Some(2048))),
        })
        .unwrap();
    let scheduler = Scheduler::start(store.clone()).unwrap();
    eprintln!("Waiting for first bytes");
    wait_until(&store, || {
        store.get(&job.id).unwrap().unwrap().downloaded > 0
    })
    .await;
    let before = store.bandwidth_limits().unwrap();
    assert!(
        change(&scheduler, None, Some(("removed".into(), None)))
            .await
            .is_err()
    );
    assert_eq!(store.bandwidth_limits().unwrap(), before);
    scheduler.send(Command::Pause(job.id.clone())).unwrap();
    // Commands run in order, so this rejected update is answered only after the
    // pause cancelled the worker. Scheduler and disk delays under load let capped
    // bytes arrive before that point, but a prompt pause stops the transfer
    // without waiting for the limiter to release more than one grant.
    assert!(
        change(&scheduler, None, Some(("removed".into(), None)))
            .await
            .is_err()
    );
    let cancelled = written(&store, &job);
    eprintln!("Waiting for pause");
    wait_until(&store, || {
        store.get(&job.id).unwrap().unwrap().status == Status::Paused
    })
    .await;
    let late = written(&store, &job) - cancelled;
    assert!(late <= 1024, "{late} capped bytes arrived after the pause");
    let partial = store.get(&job.id).unwrap().unwrap().downloaded;
    drop(scheduler);
    let scheduler = Scheduler::start(store.clone()).unwrap();
    assert_eq!(store.bandwidth_limits().unwrap(), before);
    assert_eq!(store.get(&job.id).unwrap().unwrap().downloaded, partial);
    scheduler.send(Command::Resume(job.id.clone())).unwrap();
    eprintln!("Waiting for resume");
    wait_until(&store, || {
        store.get(&job.id).unwrap().unwrap().status == Status::Downloading
    })
    .await;
    let remaining = fixture.data.len() as u64 - store.get(&job.id).unwrap().unwrap().downloaded;
    let start = Instant::now();
    change(&scheduler, None, Some((job.id.clone(), None)))
        .await
        .unwrap();
    eprintln!("Waiting for completion");
    wait_until(&store, || {
        store.get(&job.id).unwrap().unwrap().status == Status::Complete
    })
    .await;
    // The 1 KiB/s global cap needs about two minutes for the remaining bytes.
    // Finishing within half of that shows the active transfer lost its cap,
    // and leaves room for the slow flushes and commits seen under load.
    let capped = Duration::from_secs_f64(remaining as f64 / 1024.0);
    assert!(
        start.elapsed() < capped / 2,
        "Uncapped completion took {:?} of {capped:?}; job={:?}; scheduler errors={:?}",
        start.elapsed(),
        store.get(&job.id).unwrap(),
        scheduler.errors.try_iter().collect::<Vec<_>>()
    );
    let saved = store.get(&job.id).unwrap().unwrap();
    assert_eq!(
        std::fs::read(saved.saved_path.unwrap()).unwrap(),
        *fixture.data
    );
    assert_eq!(store.bandwidth_limits().unwrap(), Limits::default());
    scheduler.send(Command::Remove(job.id.clone())).unwrap();
    eprintln!("Waiting for removal");
    wait_until(&store, || store.get(&job.id).unwrap().is_none()).await;
    assert!(scheduler.errors.try_recv().is_err());
}
