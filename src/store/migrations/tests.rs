use super::*;
use crate::{
    model::{Job, Status},
    paths::AppPaths,
    store::Store,
};
use rusqlite::params;

fn paths(root: &std::path::Path) -> AppPaths {
    AppPaths {
        data: root.join("data"),
        downloads: root.join("downloads"),
    }
}

#[test]
fn legacy_upgrade_preserves_job_json_settings_parts_and_downloads() {
    let root = tempfile::tempdir().unwrap();
    let paths = paths(root.path());
    paths.prepare().unwrap();
    let conn = Connection::open(paths.data.join("queue.sqlite3")).unwrap();
    baseline(&conn).unwrap();
    let mut job = Job::new(
        "https://example.invalid/resume.bin",
        None,
        paths.downloads.clone(),
        "legacy-request".into(),
    )
    .unwrap();
    job.status = Status::Paused;
    job.downloaded = 7;
    let mut data = serde_json::to_value(&job).unwrap();
    data["legacy_extension"] = serde_json::json!({"keep":"unchanged"});
    let data = serde_json::to_string(&data).unwrap();
    conn.execute(
        "INSERT INTO jobs VALUES(?1,?2,?3,?4)",
        params![
            job.id,
            job.request_id,
            i64::try_from(job.created).unwrap(),
            data
        ],
    )
    .unwrap();
    conn.execute(
        "UPDATE interface_preferences SET data=?1",
        [r#"{"language":"en","custom":true}"#],
    )
    .unwrap();
    let part = paths.parts(&job.id).unwrap();
    std::fs::create_dir_all(&part).unwrap();
    std::fs::write(part.join("segment-0.part"), b"partial").unwrap();
    let finished = paths.downloads.join("existing.bin");
    std::fs::write(&finished, b"existing user download").unwrap();
    drop(conn);
    for _ in 0..2 {
        let store = Store::open(paths.clone()).unwrap();
        let restored = store.get(&job.id).unwrap().unwrap();
        assert_eq!((restored.status, restored.downloaded), (Status::Paused, 7));
        assert_eq!(store.job_queue(&job.id).unwrap().as_deref(), Some("main"));
        let conn = store.connect().unwrap();
        assert_eq!(
            conn.query_row("SELECT data FROM jobs WHERE id=?1", [&job.id], |r| r
                .get::<_, String>(0))
                .unwrap(),
            data
        );
        assert_eq!(
            conn.query_row("SELECT data FROM interface_preferences", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            r#"{"language":"en","custom":true}"#
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM profile_migrations", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    assert_eq!(
        std::fs::read(part.join("segment-0.part")).unwrap(),
        b"partial"
    );
    assert_eq!(std::fs::read(finished).unwrap(), b"existing user download");
}

#[test]
fn unregistered_old_writer_cannot_insert_update_or_delete_any_profile_table() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(paths(root.path())).unwrap();
    let legacy = Connection::open(store.paths.data.join("queue.sqlite3")).unwrap();
    assert_eq!(
        legacy
            .query_row(
                "SELECT name FROM download_queues WHERE id='main'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "Main queue"
    );
    for sql in [
        "INSERT OR IGNORE INTO download_queues(id,name,max_active,running) VALUES('main','Main queue',3,1)",
        "UPDATE download_queues SET name='lost' WHERE id='main'",
        "DELETE FROM download_queues WHERE id='main'",
        "UPDATE profile_format SET min_writer_version=0",
        "INSERT INTO control_requests VALUES('old','{}',0,'pending',NULL,NULL)",
    ] {
        let error = legacy.execute(sql, []).unwrap_err().to_string();
        assert!(
            error.contains("xdownloader_writer_version"),
            "{sql}: {error}"
        );
    }
    assert_eq!(store.queues().unwrap()[0].name, "Main queue");
    // Current writer still works while the old connection is open.
    store.create_queue("Current writer").unwrap();
}

#[test]
fn future_profile_refuses_open_and_an_already_open_writer_cannot_overwrite_it() {
    for (column, current) in [
        ("schema_version", SCHEMA_VERSION),
        ("data_version", DATA_VERSION),
        ("min_writer_version", WRITER_VERSION),
    ] {
        let root = tempfile::tempdir().unwrap();
        let paths = paths(root.path());
        let store = Store::open(paths.clone()).unwrap();
        let conn = store.connect().unwrap();
        conn.execute(
            &format!("UPDATE profile_format SET {column}=?1"),
            [current + 1],
        )
        .unwrap();
        let before: String = conn
            .query_row("SELECT group_concat(sql) FROM sqlite_schema", [], |r| {
                r.get(0)
            })
            .unwrap();
        let error = Store::open(paths)
            .err()
            .expect("future format must refuse")
            .to_string();
        assert!(error.contains("newer XDownloader"), "{error}");
        assert!(store.create_queue("Forbidden").is_err());
        // Reuse a connection established before the format change as well.
        assert!(
            conn.execute("UPDATE download_queues SET name='lost'", [])
                .is_err()
        );
        assert_eq!(
            before,
            conn.query_row("SELECT group_concat(sql) FROM sqlite_schema", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap()
        );
    }
}

#[test]
fn failed_late_migration_rolls_back_all_earlier_schema_and_data_changes() {
    let root = tempfile::tempdir().unwrap();
    let paths = paths(root.path());
    paths.prepare().unwrap();
    let conn = Connection::open(paths.data.join("queue.sqlite3")).unwrap();
    conn.execute_batch("CREATE TABLE desktop_preferences(broken INTEGER); INSERT INTO desktop_preferences VALUES(42);").unwrap();
    assert!(Store::open(paths.clone()).is_err());
    let tables: String = conn
        .query_row(
            "SELECT group_concat(name) FROM sqlite_schema WHERE type='table'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tables, "desktop_preferences");
    assert_eq!(
        conn.query_row("SELECT broken FROM desktop_preferences", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        42
    );
    conn.execute("DROP TABLE desktop_preferences", []).unwrap();
    drop(conn);
    assert!(Store::open(paths).is_ok());
}

#[test]
fn concurrent_openers_apply_the_migration_once() {
    let root = tempfile::tempdir().unwrap();
    let paths = paths(root.path());
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let paths = paths.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                Store::open(paths).unwrap()
            })
        })
        .collect();
    for worker in workers {
        let store = worker.join().unwrap();
        assert_eq!(
            store
                .connect()
                .unwrap()
                .query_row("SELECT count(*) FROM profile_migrations", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}
