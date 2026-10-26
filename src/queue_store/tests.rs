use super::*;
use crate::paths::AppPaths;

#[test]
fn legacy_queue_limit_migration_preserves_jobs_schedules_and_references() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    };
    paths.prepare().unwrap();
    // Reproduce the real old schema, including dependent tables populated by
    // the previous app. A foreign-key check alone would miss cascade deletion.
    let conn = Connection::open(paths.data.join("queue.sqlite3")).unwrap();
    conn.execute_batch(
        "CREATE TABLE download_queues(
        id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE COLLATE NOCASE,
        max_active INTEGER NOT NULL CHECK(max_active BETWEEN 1 AND 8),
        running INTEGER NOT NULL, scheduled_at INTEGER);
        INSERT INTO download_queues VALUES('main','Main queue',3,0,NULL);",
    )
    .unwrap();
    drop(conn);
    // Create a complete modern profile, then restore just the legacy constraint
    // in an isolated test database using the same safe replacement sequence.
    let store = Store::open(paths.clone()).unwrap();
    let queue = store.create_sync_queue("Scheduled sync").unwrap();
    let recurring = RecurringSchedule::new(127, "23:00", "07:00").unwrap();
    store
        .schedule_recurring_queue(&queue.id, &recurring, chrono::Utc::now().timestamp())
        .unwrap();
    let mut job = Job::new(
        "https://example.invalid/keep.bin",
        None,
        paths.downloads.clone(),
        "retained-job".into(),
    )
    .unwrap();
    job.status = Status::Paused;
    store.enqueue_batch(&[job.clone()], &queue.id).unwrap();
    let before = store.queues().unwrap();
    let conn = store.connect().unwrap();
    conn.pragma_update(None, "foreign_keys", false).unwrap();
    conn.execute_batch("BEGIN IMMEDIATE;
        CREATE TABLE legacy_queues(id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE COLLATE NOCASE,
        max_active INTEGER NOT NULL CHECK(max_active BETWEEN 1 AND 8), running INTEGER NOT NULL, scheduled_at INTEGER);
        INSERT INTO legacy_queues SELECT * FROM download_queues;
        DROP TABLE download_queues;
        ALTER TABLE legacy_queues RENAME TO download_queues;
        COMMIT;").unwrap();
    drop(conn);
    drop(store);
    let store = Store::open(paths.clone()).unwrap();
    assert_eq!(store.queues().unwrap(), before);
    assert_eq!(store.queue_jobs(&queue.id).unwrap()[0].id, job.id);
    for limit in [1, 20, 30, 32] {
        store.set_queue_limit(&queue.id, limit).unwrap();
        assert_eq!(
            store
                .queues()
                .unwrap()
                .iter()
                .find(|q| q.id == queue.id)
                .unwrap()
                .max_active,
            limit
        );
    }
    for limit in [0, 33, usize::MAX] {
        assert!(store.set_queue_limit(&queue.id, limit).is_err());
    }
    let conn = store.connect().unwrap();
    assert!(
        conn.execute(
            "UPDATE download_queues SET max_active=33 WHERE id=?1",
            [&queue.id]
        )
        .is_err()
    );
    assert_eq!(
        conn.query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(
        conn.query_row("PRAGMA foreign_key_check", [], |row| row
            .get::<_, String>(0))
            .optional()
            .unwrap()
            .is_none()
    );
    drop(conn);
    drop(store);
    let reopened = Store::open(paths).unwrap();
    assert_eq!(reopened.queues().unwrap()[1].max_active, 32);
    assert_eq!(reopened.queue_jobs(&queue.id).unwrap()[0].id, job.id);
    drop(reopened);
    temp.close().unwrap();
}
