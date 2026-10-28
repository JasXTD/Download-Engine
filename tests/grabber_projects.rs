#[path = "../src/desktop/fixture_database.rs"]
mod fixture_database;
#[path = "support/legacy_profile.rs"]
mod legacy_profile;
use download_engine::{
    browser_batch::Destination,
    grabber_projects::{MAX_PROJECTS, Settings},
    model::{Job, Status},
    paths::AppPaths,
    queue::MAIN_QUEUE,
    store::Store,
};

fn fixture() -> (tempfile::TempDir, Store) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    (temp, store)
}

fn settings() -> Settings {
    Settings {
        url: "https://example.test/manuals/?lang=en".into(),
        extensions: "pdf, zip".into(),
        depth: 3,
        page_limit: 42,
        queue: MAIN_QUEUE.into(),
        destination: Destination::Documents,
    }
}

#[test]
fn settings_survive_restart_update_rename_copy_and_delete_without_creating_jobs() {
    let (_temp, store) = fixture();
    let queue = store.create_queue("Manuals").unwrap();
    let settings = Settings {
        queue: queue.id.clone(),
        ..settings()
    };
    let original = store
        .create_grabber_project("  Product manuals  ", &settings)
        .unwrap();
    assert_eq!(original.name, "Product manuals");
    let paths = store.paths.clone();
    drop(store);
    let store = Store::open(paths).unwrap();
    assert_eq!(
        store.grabber_projects().unwrap().as_slice(),
        std::slice::from_ref(&original)
    );
    let changed = Settings {
        depth: 0,
        page_limit: 1,
        destination: Destination::Downloads,
        ..settings.clone()
    };
    let updated = store
        .update_grabber_project(&original, "New manuals", &changed)
        .unwrap();
    assert_eq!(updated.id, original.id);
    assert_eq!(updated.revision, 2);
    assert_eq!(updated.settings, changed);
    let copy = store
        .create_grabber_project("Manuals copy", &updated.settings)
        .unwrap();
    assert_ne!(copy.id, original.id);
    store.delete_grabber_project(&updated).unwrap();
    assert_eq!(store.grabber_projects().unwrap(), [copy]);
    assert!(store.list().unwrap().is_empty());
    assert!(
        !store
            .queues()
            .unwrap()
            .iter()
            .find(|q| q.id == queue.id)
            .unwrap()
            .running
    );
}

#[test]
fn invalid_settings_duplicate_names_and_failed_updates_preserve_saved_projects() {
    let (_temp, store) = fixture();
    let original = store
        .create_grabber_project("Manuals", &settings())
        .unwrap();
    for name in ["", "  ", "bad\nname", &"x".repeat(81), "MANUALS"] {
        assert!(
            store.create_grabber_project(name, &settings()).is_err(),
            "{name:?}"
        );
    }
    let bad = [
        Settings {
            url: "file:///C:/private".into(),
            ..settings()
        },
        Settings {
            url: "https://user:secret@example.test/".into(),
            ..settings()
        },
        Settings {
            url: "x".repeat(16385),
            ..settings()
        },
        Settings {
            extensions: "*".into(),
            ..settings()
        },
        Settings {
            extensions: "".into(),
            ..settings()
        },
        Settings {
            depth: 6,
            ..settings()
        },
        Settings {
            page_limit: 0,
            ..settings()
        },
        Settings {
            page_limit: 201,
            ..settings()
        },
        Settings {
            queue: "missing".into(),
            ..settings()
        },
    ];
    for invalid in bad {
        assert!(store.create_grabber_project("Invalid", &invalid).is_err());
        assert!(
            store
                .update_grabber_project(&original, "Invalid", &invalid)
                .is_err()
        );
    }
    let other = store.create_grabber_project("Other", &settings()).unwrap();
    assert!(
        store
            .update_grabber_project(&original, "other", &settings())
            .is_err()
    );
    assert_eq!(store.grabber_projects().unwrap(), [original, other]);
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn stale_edits_and_deletions_do_not_overwrite_newer_saved_settings() {
    let (_temp, store) = fixture();
    let stale = store
        .create_grabber_project("Manuals", &settings())
        .unwrap();
    let other = Store::open(store.paths.clone()).unwrap();
    let current = other
        .update_grabber_project(
            &stale,
            "New name",
            &Settings {
                depth: 5,
                ..settings()
            },
        )
        .unwrap();
    assert!(
        store
            .update_grabber_project(&stale, "Old copy", &settings())
            .unwrap_err()
            .to_string()
            .contains("changed or was deleted")
    );
    assert!(store.delete_grabber_project(&stale).is_err());
    assert_eq!(
        store.grabber_projects().unwrap().as_slice(),
        std::slice::from_ref(&current)
    );
    other.delete_grabber_project(&current).unwrap();
    assert!(
        store
            .update_grabber_project(&current, "Revived", &settings())
            .is_err()
    );
    assert!(store.delete_grabber_project(&current).is_err());
    assert!(store.grabber_projects().unwrap().is_empty());
}

#[test]
fn deleting_a_queue_preserves_project_but_requires_explicit_replacement_to_save() {
    let (_temp, store) = fixture();
    let queue = store.create_queue("Temporary").unwrap();
    let original = store
        .create_grabber_project(
            "Manuals",
            &Settings {
                queue: queue.id.clone(),
                ..settings()
            },
        )
        .unwrap();
    store.delete_queue(&queue.id).unwrap();
    let reopened = Store::open(store.paths.clone()).unwrap();
    assert_eq!(
        reopened.grabber_projects().unwrap().as_slice(),
        std::slice::from_ref(&original)
    );
    assert!(
        reopened
            .update_grabber_project(&original, "Manuals", &original.settings)
            .is_err()
    );
    let updated = reopened
        .update_grabber_project(&original, "Manuals", &settings())
        .unwrap();
    assert_eq!(updated.settings.queue, MAIN_QUEUE);
    assert!(reopened.list().unwrap().is_empty());
}

#[test]
fn migration_and_project_edits_preserve_existing_downloads_and_preferences() {
    let (_temp, store) = fixture();
    let mut job = Job::new(
        "https://example.test/file.zip",
        None,
        store.paths.downloads.clone(),
        "legacy".into(),
    )
    .unwrap();
    job.status = Status::Paused;
    store.enqueue(&job).unwrap();
    let prefs = store.interface_preferences().unwrap();
    let paths = store.paths.clone();
    drop(store);
    let conn = crate::fixture_database::open(paths.data.join("queue.sqlite3")).unwrap();
    conn.execute("DROP TABLE grabber_projects", []).unwrap();
    legacy_profile::unversioned_fixture(&conn);
    drop(conn);
    let migrated = Store::open(paths.clone()).unwrap();
    assert!(migrated.grabber_projects().unwrap().is_empty());
    let project = migrated
        .create_grabber_project("First project", &settings())
        .unwrap();
    job.downloaded = 17;
    migrated.save(&job).unwrap();
    let reopened = Store::open(paths).unwrap();
    assert_eq!(reopened.grabber_projects().unwrap(), [project]);
    assert_eq!(reopened.get(&job.id).unwrap().unwrap().downloaded, 17);
    assert_eq!(reopened.interface_preferences().unwrap(), prefs);
    assert_eq!(
        reopened.job_queue(&job.id).unwrap().as_deref(),
        Some(MAIN_QUEUE)
    );
}

#[test]
fn project_limit_is_atomic_across_store_instances_and_does_not_block_updates() {
    let (_temp, store) = fixture();
    for i in 0..MAX_PROJECTS - 1 {
        store
            .create_grabber_project(&format!("Project {i:03}"), &settings())
            .unwrap();
    }
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let tasks: Vec<_> = (0..2)
        .map(|i| {
            let store = store.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.create_grabber_project(&format!("Last {i}"), &settings())
            })
        })
        .collect();
    let outcomes: Vec<_> = tasks.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(outcomes.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(store.grabber_projects().unwrap().len(), MAX_PROJECTS);
    let project = store.grabber_projects().unwrap().remove(0);
    let updated = store
        .update_grabber_project(&project, "Renamed at limit", &settings())
        .unwrap();
    store.delete_grabber_project(&updated).unwrap();
    assert!(
        store
            .create_grabber_project("Replacement", &settings())
            .is_ok()
    );
}

#[test]
fn stored_invalid_urls_are_rejected_and_folder_choices_use_stable_names() {
    let (_temp, store) = fixture();
    let saved = store
        .create_grabber_project("Manuals", &settings())
        .unwrap();
    let conn = crate::fixture_database::open(store.paths.data.join("queue.sqlite3")).unwrap();
    let raw: String = conn
        .query_row("SELECT settings FROM grabber_projects", [], |r| r.get(0))
        .unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(json["destination"], "documents");
    json["url"] = "file:///C:/private".into();
    conn.execute(
        "UPDATE grabber_projects SET settings=?1 WHERE id=?2",
        rusqlite::params![json.to_string(), saved.id],
    )
    .unwrap();
    assert!(store.grabber_projects().is_err());
    assert!(store.list().unwrap().is_empty());
}
