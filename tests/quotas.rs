#[path = "../src/desktop/fixture_database.rs"]
mod fixture_database;
#[path = "support/legacy_profile.rs"]
mod legacy_profile;
use download_engine::{
    paths::AppPaths,
    quota::{DAY, HOUR, Limits, MAX_LIMIT, MIN_LIMIT},
    store::Store,
};

fn setup() -> (tempfile::TempDir, Store) {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: root.path().join("data"),
        downloads: root.path().join("downloads"),
    })
    .unwrap();
    (root, store)
}
#[test]
fn disabled_by_default_and_usage_windows_survive_restarts_and_edits() {
    let (_root, store) = setup();
    assert_eq!(store.quota_status_at(100).unwrap(), Default::default());
    assert_eq!(
        store.record_quota_bytes_at(500, 100).unwrap().hourly_used,
        0
    );
    let limits = Limits {
        hourly: Some(MIN_LIMIT),
        daily: Some(2 * MIN_LIMIT),
    };
    let initial = store.set_quota_limits_at(limits, 100).unwrap();
    assert_eq!(
        (initial.hour_start, initial.day_start),
        (Some(100), Some(100))
    );
    let full = store.record_quota_bytes_at(MIN_LIMIT, 110).unwrap();
    assert!(full.blocked());
    assert_eq!(full.resumes_at(), Some(100 + HOUR));
    let reopened = Store::open(store.paths.clone()).unwrap();
    assert_eq!(reopened.quota_status_at(200).unwrap(), full);
    let larger = Limits {
        hourly: Some(2 * MIN_LIMIT),
        ..limits
    };
    assert!(!reopened.set_quota_limits_at(larger, 200).unwrap().blocked());
    assert!(reopened.set_quota_limits_at(limits, 200).unwrap().blocked());
    let disabled = reopened
        .set_quota_limits_at(Limits::default(), 200)
        .unwrap();
    assert!(!disabled.blocked());
    assert_eq!(disabled.hourly_used, MIN_LIMIT);
    assert_eq!(
        reopened
            .record_quota_bytes_at(MIN_LIMIT, 250)
            .unwrap()
            .daily_used,
        MIN_LIMIT
    );
    assert!(reopened.set_quota_limits_at(limits, 300).unwrap().blocked());
}
#[test]
fn each_window_resets_independently_and_clock_rollback_never_refunds() {
    let (_root, store) = setup();
    let limits = Limits {
        hourly: Some(MIN_LIMIT),
        daily: Some(2 * MIN_LIMIT),
    };
    store.set_quota_limits_at(limits, 100).unwrap();
    let full = store.record_quota_bytes_at(2 * MIN_LIMIT, 110).unwrap();
    assert_eq!(full.resumes_at(), Some(100 + DAY));
    assert_eq!(store.quota_status_at(50).unwrap(), full);
    assert_eq!(store.quota_status_at(100 + HOUR - 1).unwrap(), full);
    let hour = store.quota_status_at(100 + HOUR).unwrap();
    assert_eq!(hour.hourly_used, 0);
    assert_eq!(hour.daily_used, 2 * MIN_LIMIT);
    assert!(hour.blocked());
    assert_eq!(hour.resumes_at(), Some(100 + DAY));
    let leap = store
        .quota_status_at(100 + 5 * DAY + 2 * HOUR + 11)
        .unwrap();
    assert!(!leap.blocked());
    assert_eq!((leap.hourly_used, leap.daily_used), (0, 0));
    assert_eq!(leap.day_start, Some(100 + 5 * DAY));
    assert_eq!(leap.hour_start, Some(100 + 5 * DAY + 2 * HOUR));
}
#[test]
fn enabling_second_limit_uses_already_tracked_bytes_and_invalid_changes_are_atomic() {
    let (_root, store) = setup();
    store
        .set_quota_limits_at(
            Limits {
                hourly: Some(MIN_LIMIT),
                daily: None,
            },
            100,
        )
        .unwrap();
    store.record_quota_bytes_at(MIN_LIMIT, 101).unwrap();
    let before = store
        .set_quota_limits_at(
            Limits {
                hourly: None,
                daily: Some(MIN_LIMIT),
            },
            102,
        )
        .unwrap();
    assert!(before.blocked());
    for value in [0, MIN_LIMIT - 1, MAX_LIMIT + 1, u64::MAX] {
        assert!(
            store
                .set_quota_limits_at(
                    Limits {
                        hourly: Some(value),
                        daily: None
                    },
                    102
                )
                .is_err()
        );
        assert_eq!(store.quota_status_at(102).unwrap(), before);
    }
    assert!(store.record_quota_bytes_at(u64::MAX, 102).is_err());
    assert!(store.quota_status_at(-1).is_err());
    assert!(store.quota_status_at(i64::MAX).is_err());
    assert_eq!(store.quota_status_at(102).unwrap(), before);
}
#[test]
fn concurrent_stores_do_not_lose_received_bytes() {
    let (_root, store) = setup();
    store
        .set_quota_limits_at(
            Limits {
                hourly: None,
                daily: Some(MAX_LIMIT),
            },
            100,
        )
        .unwrap();
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let separate = Store::open(store.paths.clone()).unwrap();
            scope.spawn(move || {
                for _ in 0..32 {
                    separate.record_quota_bytes_at(16384, 101).unwrap();
                }
            });
        }
    });
    let status = store.quota_status_at(101).unwrap();
    assert_eq!(
        (status.hourly_used, status.daily_used),
        (4 * MIN_LIMIT, 4 * MIN_LIMIT)
    );
}
#[test]
fn quota_table_migrates_without_changing_old_jobs_or_other_settings() {
    let (_root, store) = setup();
    let job = download_engine::model::Job::new(
        "https://example.com/a.bin",
        None,
        store.paths.downloads.clone(),
        "migration".into(),
    )
    .unwrap();
    store.enqueue(&job).unwrap();
    let conn = crate::fixture_database::open(store.paths.data.join("queue.sqlite3")).unwrap();
    let before: String = conn
        .query_row("SELECT data FROM jobs", [], |r| r.get(0))
        .unwrap();
    conn.execute("DROP TABLE data_quotas", []).unwrap();
    legacy_profile::unversioned_fixture(&conn);
    drop(conn);
    let reopened = Store::open(store.paths.clone()).unwrap();
    assert_eq!(reopened.quota_status().unwrap().limits, Limits::default());
    let conn = crate::fixture_database::open(store.paths.data.join("queue.sqlite3")).unwrap();
    let after: String = conn
        .query_row("SELECT data FROM jobs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(before, after);
    assert!(!reopened.get(&job.id).unwrap().unwrap().quota_waiting);
}
