#[path = "../src/desktop/fixture_database.rs"]
mod fixture_database;
use download_engine::{
    model::{Job, Status},
    paths::AppPaths,
    queue::{MAIN_QUEUE, batch_jobs, local_schedule},
    store::Store,
};
use rusqlite::params;

fn setup() -> (tempfile::TempDir, Store) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("files"),
    })
    .unwrap();
    (temp, store)
}
fn job(store: &Store, name: &str) -> Job {
    Job::new(
        &format!("https://example.com/{name}"),
        None,
        store.paths.downloads.clone(),
        name.into(),
    )
    .unwrap()
}
fn ids(store: &Store, queue: &str) -> Vec<String> {
    store
        .queue_jobs(queue)
        .unwrap()
        .into_iter()
        .map(|j| j.id)
        .collect()
}

#[test]
fn legacy_history_migrates_in_original_order_once() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("files"),
    };
    paths.prepare().unwrap();
    let conn = crate::fixture_database::open(paths.data.join("queue.sqlite3")).unwrap();
    conn.execute_batch(
        "CREATE TABLE jobs(id TEXT PRIMARY KEY,request_id TEXT UNIQUE,created INTEGER,data TEXT);",
    )
    .unwrap();
    let mut jobs = Vec::new();
    for (name, created) in [("new.bin", 200), ("old.bin", 100)] {
        let mut j = Job::new(
            &format!("https://example.com/{name}"),
            None,
            paths.downloads.clone(),
            name.into(),
        )
        .unwrap();
        j.status = Status::Paused;
        j.downloaded = 123;
        j.created = created;
        conn.execute(
            "INSERT INTO jobs VALUES(?1,?2,?3,?4)",
            params![
                j.id,
                j.request_id,
                created as i64,
                serde_json::to_string(&j).unwrap()
            ],
        )
        .unwrap();
        jobs.push(j);
    }
    drop(conn);
    let store = Store::open(paths.clone()).unwrap();
    assert_eq!(
        ids(&store, MAIN_QUEUE),
        vec![jobs[1].id.clone(), jobs[0].id.clone()]
    );
    assert_eq!(store.get(&jobs[0].id).unwrap().unwrap().downloaded, 123);
    store.reorder_job(&jobs[0].id, true).unwrap();
    store.set_queue_running(MAIN_QUEUE, false).unwrap();
    let reopened = Store::open(paths).unwrap();
    assert_eq!(
        ids(&reopened, MAIN_QUEUE),
        vec![jobs[0].id.clone(), jobs[1].id.clone()]
    );
    assert!(!reopened.queues().unwrap()[0].running);
}

#[test]
fn batch_validation_deduplication_and_transaction_rollback() {
    let (_temp, store) = setup();
    let later = store.create_queue("Later").unwrap();
    let parsed = batch_jobs("https://example.com/a.zip\n\nhttps://example.com/a.zip#fragment\nhttps://example.com/b.zip", &store.paths.downloads).unwrap();
    assert_eq!(parsed.len(), 2);
    let bad = batch_jobs(
        "https://example.com/a.zip\nsftp://example.com/b.zip",
        &store.paths.downloads,
    )
    .unwrap_err();
    assert!(format!("{bad:#}").contains("Line 2"));
    assert!(store.list().unwrap().is_empty());
    store.enqueue_batch(&parsed, &later.id).unwrap();
    let fresh = job(&store, "fresh.zip");
    let mut conflict = parsed[0].clone();
    conflict.url = "https://example.com/different.zip".into();
    assert!(
        store
            .enqueue_batch(&[fresh.clone(), conflict], &later.id)
            .is_err()
    );
    assert!(store.get(&fresh.id).unwrap().is_none());
    assert_eq!(
        ids(&store, &later.id),
        parsed.iter().map(|j| j.id.clone()).collect::<Vec<_>>()
    );
    assert!(store.enqueue_batch(&[fresh], "missing-queue").is_err());
}

#[test]
fn metadata_survives_progress_saves_moves_and_reopen() {
    let (_temp, store) = setup();
    let later = store.create_queue("Later").unwrap();
    let a = job(&store, "a.zip");
    let mut b = job(&store, "b.zip");
    store
        .enqueue_batch(&[a.clone(), b.clone()], MAIN_QUEUE)
        .unwrap();
    store.reorder_job(&b.id, true).unwrap();
    b.downloaded = 456;
    store.save(&b).unwrap();
    assert_eq!(ids(&store, MAIN_QUEUE), vec![b.id.clone(), a.id.clone()]);
    store.move_job(&b.id, &later.id).unwrap();
    store.save(&b).unwrap();
    let reopened = Store::open(store.paths.clone()).unwrap();
    assert_eq!(ids(&reopened, &later.id), vec![b.id.clone()]);
    assert_eq!(reopened.get(&b.id).unwrap().unwrap().downloaded, 456);
    reopened.remove(&b.id).unwrap();
    assert!(reopened.job_queue(&b.id).unwrap().is_none());
    reopened.delete_queue(&later.id).unwrap();
}

#[test]
fn browser_handoffs_keep_default_membership_and_durable_receipts() {
    let (_temp, store) = setup();
    store.set_queue_running(MAIN_QUEUE, false).unwrap();
    let manual = job(&store, "manual.zip");
    store.enqueue(&manual).unwrap();
    store.enqueue(&manual).unwrap();
    let captured = job(&store, "captured.zip");
    store.prepare_capture(&captured).unwrap();
    assert_eq!(ids(&store, MAIN_QUEUE), vec![manual.id.clone()]);
    store.commit_capture(&captured.request_id).unwrap();
    store.commit_capture(&captured.request_id).unwrap();
    assert_eq!(
        ids(&store, MAIN_QUEUE),
        vec![manual.id, captured.id.clone()]
    );
    assert_eq!(
        store.get(&captured.id).unwrap().unwrap().status,
        Status::Queued
    );
    store.remove(&captured.id).unwrap();
    assert_eq!(
        store.commit_capture(&captured.request_id).unwrap().job_id,
        Some(captured.id)
    );
}

#[test]
fn queue_names_limits_and_deletion_are_validated() {
    let (_temp, store) = setup();
    assert!(store.create_queue("  ").is_err());
    assert!(store.create_queue("a\nb").is_err());
    assert!(store.create_queue(&"x".repeat(65)).is_err());
    let later = store.create_queue(" Later ").unwrap();
    assert_eq!(later.name, "Later");
    assert!(!later.running);
    assert!(store.create_queue("later").is_err());
    assert!(store.set_queue_limit(&later.id, 0).is_err());
    let maximum = download_engine::queue::MAX_ACTIVE;
    store.set_queue_limit(&later.id, maximum).unwrap();
    assert_eq!(store.queues().unwrap()[1].max_active, maximum);
    assert!(store.set_queue_limit(&later.id, maximum + 1).is_err());
    assert!(store.delete_queue(MAIN_QUEUE).is_err());
    store
        .enqueue_batch(&[job(&store, "one.zip")], &later.id)
        .unwrap();
    assert!(store.delete_queue(&later.id).is_err());
    store.rename_queue(&later.id, "Tonight").unwrap();
    assert_eq!(store.queues().unwrap()[1].name, "Tonight");
}

#[test]
fn schedules_are_persistent_one_shot_and_manual_controls_clear_them() {
    let (_temp, store) = setup();
    let q = store.create_queue("Tonight").unwrap();
    store.schedule_queue(&q.id, 200, 100).unwrap();
    let reopened = Store::open(store.paths.clone()).unwrap();
    assert_eq!(reopened.apply_queue_schedules(199).unwrap(), 0);
    assert_eq!(reopened.apply_queue_schedules(201).unwrap(), 1);
    assert_eq!(reopened.apply_queue_schedules(202).unwrap(), 0);
    let saved = reopened.queues().unwrap().remove(1);
    assert!(saved.running);
    assert_eq!(saved.scheduled_at, None);
    store.schedule_queue(&q.id, 400, 300).unwrap();
    store.clear_schedule(&q.id).unwrap();
    assert_eq!(store.apply_queue_schedules(500).unwrap(), 0);
    assert!(!store.queues().unwrap()[1].running);
    store.schedule_queue(&q.id, 700, 600).unwrap();
    store.set_queue_running(&q.id, false).unwrap();
    assert_eq!(store.apply_queue_schedules(800).unwrap(), 0);
    assert!(store.schedule_queue(&q.id, 100, 100).is_err());
}

#[test]
fn schedule_input_uses_local_time_and_rejects_invalid_dates() {
    use chrono::{Local, TimeZone, Timelike};
    let next = (Local::now() + chrono::Duration::days(2))
        .with_second(0)
        .unwrap()
        .with_nanosecond(0)
        .unwrap();
    let at = local_schedule(
        &next.format("%Y-%m-%d").to_string(),
        &next.format("%H:%M").to_string(),
        Local::now().timestamp(),
    )
    .unwrap();
    assert_eq!(Local.timestamp_opt(at, 0).single().unwrap(), next);
    for (date, time) in [
        ("2027-02-30", "10:00"),
        ("tomorrow", "10:00"),
        ("2027-10-20", "25:10"),
        ("2000-01-01", "00:00"),
    ] {
        assert!(local_schedule(date, time, Local::now().timestamp()).is_err());
    }
}

#[test]
fn interrupted_queue_pause_recovers_without_resuming_individual_pauses() {
    let (_temp, store) = setup();
    let mut queued_pause = job(&store, "queue-paused.zip");
    let mut user_pause = job(&store, "user-paused.zip");
    let mut interrupted = job(&store, "interrupted.zip");
    queued_pause.status = Status::Downloading;
    user_pause.status = Status::Paused;
    interrupted.status = Status::Downloading;
    store
        .enqueue_batch(
            &[
                queued_pause.clone(),
                user_pause.clone(),
                interrupted.clone(),
            ],
            MAIN_QUEUE,
        )
        .unwrap();
    let conn = crate::fixture_database::open(store.paths.data.join("queue.sqlite3")).unwrap();
    conn.execute(
        "UPDATE job_queues SET paused_by_queue=1 WHERE job_id=?1",
        [&queued_pause.id],
    )
    .unwrap();
    store.recover().unwrap();
    assert_eq!(
        store.get(&queued_pause.id).unwrap().unwrap().status,
        Status::Queued
    );
    assert_eq!(
        store.get(&user_pause.id).unwrap().unwrap().status,
        Status::Paused
    );
    assert_eq!(
        store.get(&interrupted.id).unwrap().unwrap().status,
        Status::Paused
    );
}

#[test]
fn concurrent_host_opens_and_enqueues_never_reset_queue_order() {
    let (_temp, store) = setup();
    let q = store.create_queue("Later").unwrap();
    let jobs = [job(&store, "a.zip"), job(&store, "b.zip")];
    store.enqueue_batch(&jobs, &q.id).unwrap();
    store.reorder_job(&jobs[1].id, true).unwrap();
    let handles: Vec<_> = (0..6)
        .map(|index| {
            let paths = store.paths.clone();
            std::thread::spawn(move || {
                let store = Store::open(paths).unwrap();
                store
                    .enqueue(&job(&store, &format!("browser-{index}.zip")))
                    .unwrap();
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    assert_eq!(
        ids(&store, &q.id),
        vec![jobs[1].id.clone(), jobs[0].id.clone()]
    );
    assert_eq!(ids(&store, MAIN_QUEUE).len(), 6);
}

#[test]
fn store_clones_keep_wal_active_without_blocking_readers_or_checkpoints() {
    let (_temp, store) = setup();
    let paths = store.paths.clone();
    let wal = paths.data.join("queue.sqlite3-wal");
    let mut saved = job(&store, "wal-lifetime.bin");
    store.enqueue(&saved).unwrap();
    let remaining = store.clone();
    drop(store);
    // Closing each operation's connection must not take the last-connection
    // exclusive lock and checkpoint/delete/recreate WAL while the store is live.
    assert!(wal.exists(), "A live Store lost its WAL between operations");
    let observer = crate::fixture_database::open(paths.data.join("queue.sqlite3")).unwrap();
    let mode: String = observer
        .pragma_query_value(None, "journal_mode", |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    let synchronous: u32 = observer
        .pragma_query_value(None, "synchronous", |r| r.get(0))
        .unwrap();
    assert_eq!(
        synchronous, 2,
        "Durable writes must retain FULL synchronization"
    );
    saved.status = Status::Paused;
    saved.downloaded = 1234;
    remaining.save(&saved).unwrap();
    let json: String = observer
        .query_row("SELECT data FROM jobs WHERE id=?1", [&saved.id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(serde_json::from_str::<Job>(&json).unwrap().downloaded, 1234);
    let (busy, frames, checkpointed): (i64, i64, i64) = observer
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap();
    assert_eq!(
        (busy, frames, checkpointed),
        (0, 0, 0),
        "An idle store must not pin a read transaction"
    );
    drop(observer);
    assert!(
        wal.exists(),
        "The remaining Store clone must retain WAL ownership"
    );
    assert_eq!(remaining.get(&saved.id).unwrap().unwrap().downloaded, 1234);
    drop(remaining);
    assert!(
        !wal.exists(),
        "The final Store drop must release the database handles"
    );
    let reopened = Store::open(paths).unwrap();
    assert_eq!(reopened.get(&saved.id).unwrap().unwrap().downloaded, 1234);
}
