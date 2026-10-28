#[path = "../src/desktop/fixture_database.rs"]
mod fixture_database;
use download_engine::{
    categories::{Category, DownloadFolders},
    model::{Job, Status},
    native,
    paths::AppPaths,
    queue::MAIN_QUEUE,
    store::Store,
};
use rusqlite::params;
use std::path::Path;

fn setup() -> (tempfile::TempDir, Store) {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: root.path().join("data"),
        downloads: root.path().join("downloads"),
    })
    .unwrap();
    (root, store)
}
fn create(store: &Store, name: &str, folder: Option<&Path>) -> Job {
    store
        .new_download(
            "https://example.test/download?name=misleading.exe",
            Some(name),
            folder,
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap()
}

#[test]
fn categories_use_the_sanitized_filename_and_final_extension() {
    use Category::*;
    let (_root, store) = setup();
    for (name, category) in [
        ("Guide.PDF", Documents),
        ("report.pdf.EXE", Programs),
        ("archive.tar.gz", Compressed),
        ("music.FLAC", Music),
        ("movie.MKV", Video),
        ("photo.png", Other),
        ("unknown.blob", Other),
        ("download", Other),
        (".zip", Other),
        ("../invoice.pdf", Documents),
        ("bill\u{202e}fdp.exe", Programs),
    ] {
        let job = create(&store, name, None);
        assert_eq!(job.category(), category, "{name}");
        assert_eq!(job.directory, store.paths.downloads.join(category.label()));
        assert_eq!(Path::new(&job.name).components().count(), 1);
    }
    let decoded = store
        .new_download(
            "https://example.test/guide%2EPDF?filename=setup.exe",
            None,
            None,
            "decoded".into(),
        )
        .unwrap();
    assert_eq!(decoded.category(), Documents);
    assert_eq!(decoded.name, "guide.PDF");
}

#[test]
fn default_custom_disabled_and_explicit_folders_apply_to_manual_and_batch_downloads() {
    let (root, store) = setup();
    assert_eq!(
        store.download_folders().unwrap(),
        DownloadFolders::default()
    );
    let mut rules = store.download_folders().unwrap();
    let custom = root.path().join("my documents");
    rules.overrides.insert(Category::Documents, custom.clone());
    store.save_download_folders(&rules).unwrap();
    assert_eq!(create(&store, "guide.pdf", None).directory, custom);
    let batch = store.new_batch_downloads("https://example.test/a.pdf\nhttps://example.test/b.zip\nhttps://example.test/a.pdf#fragment", None).unwrap();
    assert_eq!(batch.len(), 2);
    assert_eq!(batch[0].directory, custom);
    assert_eq!(batch[1].directory, store.paths.downloads.join("Compressed"));
    let explicit = root.path().join("chosen folder");
    assert_eq!(
        create(&store, "guide.pdf", Some(&explicit)).directory,
        explicit
    );
    assert!(
        store
            .new_batch_downloads(
                "https://example.test/a.pdf\nhttps://example.test/b.zip",
                Some(&explicit)
            )
            .unwrap()
            .iter()
            .all(|j| j.directory == explicit)
    );
    assert!(
        !custom.exists() && !explicit.exists(),
        "Choosing rules must not create or touch destination folders"
    );
    assert!(
        store
            .new_batch_downloads(
                "https://example.test/a.pdf\nsftp://example.test/b.zip",
                None
            )
            .is_err()
    );
    assert!(store.list().unwrap().is_empty());
    rules.organize = false;
    store.save_download_folders(&rules).unwrap();
    assert_eq!(
        create(&store, "guide.pdf", None).directory,
        store.paths.downloads
    );
    assert_eq!(
        create(&store, "guide.pdf", Some(&explicit)).directory,
        explicit
    );
    assert_eq!(
        store
            .new_batch_downloads("https://example.test/a.pdf", None)
            .unwrap()[0]
            .directory,
        store.paths.downloads
    );
    assert_eq!(
        Store::open(store.paths.clone())
            .unwrap()
            .download_folders()
            .unwrap(),
        rules
    );
}

#[test]
fn folder_edits_and_worker_saves_do_not_relocate_existing_jobs_or_change_browser_retries() {
    let (root, store) = setup();
    let request = serde_json::json!({"action":"add", "request_id":"browser-once", "url":"https://example.test/guide.pdf"});
    let first = native::handle(&store, &serde_json::to_vec(&request).unwrap());
    assert!(first.ok, "{}", first.message);
    let id = first.job_id.unwrap();
    let mut old = store.get(&id).unwrap().unwrap();
    let original = old.directory.clone();
    let mut rules = store.download_folders().unwrap();
    rules
        .overrides
        .insert(Category::Documents, root.path().join("new document folder"));
    store.save_download_folders(&rules).unwrap();
    old.status = Status::Paused;
    old.downloaded = 42;
    store.save(&old).unwrap();
    let retry = native::handle(&store, &serde_json::to_vec(&request).unwrap());
    assert!(retry.ok);
    assert_eq!(retry.job_id.as_deref(), Some(id.as_str()));
    assert_eq!(store.list().unwrap().len(), 1);
    assert_eq!(store.get(&id).unwrap().unwrap().directory, original);
    assert_eq!(store.download_folders().unwrap(), rules);
    assert_ne!(create(&store, "another.pdf", None).directory, original);
}

#[test]
fn legacy_history_and_existing_files_are_unchanged_when_rules_are_initialized() {
    let root = tempfile::tempdir().unwrap();
    let paths = AppPaths {
        data: root.path().join("data"),
        downloads: root.path().join("downloads"),
    };
    paths.prepare().unwrap();
    let conn = crate::fixture_database::open(paths.data.join("queue.sqlite3")).unwrap();
    conn.execute_batch(
        "CREATE TABLE jobs(id TEXT PRIMARY KEY,request_id TEXT UNIQUE,created INTEGER,data TEXT);",
    )
    .unwrap();
    let mut originals = Vec::new();
    for (i, status) in [
        Status::Queued,
        Status::Downloading,
        Status::Paused,
        Status::Complete,
        Status::Failed,
    ]
    .into_iter()
    .enumerate()
    {
        let mut job = Job::new(
            "https://example.test/legacy.pdf",
            None,
            paths.downloads.clone(),
            format!("legacy-{i}"),
        )
        .unwrap();
        job.status = status;
        job.downloaded = 123;
        if status == Status::Complete {
            job.saved_path = Some(job.directory.join(&job.name));
        }
        let data = serde_json::to_string(&job).unwrap();
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
        originals.push((job.id, data));
    }
    std::fs::write(paths.downloads.join("legacy.pdf"), b"preserved user file").unwrap();
    let store = Store::open(paths.clone()).unwrap();
    assert_eq!(
        store.download_folders().unwrap(),
        DownloadFolders::default()
    );
    assert_eq!(store.queue_jobs(MAIN_QUEUE).unwrap().len(), 5);
    for (id, original) in originals {
        let data: String = conn
            .query_row("SELECT data FROM jobs WHERE id=?1", [&id], |r| r.get(0))
            .unwrap();
        assert_eq!(data, original, "Migration rewrote an existing job");
        let job = store.get(&id).unwrap().unwrap();
        assert_eq!(job.directory, paths.downloads);
        assert_eq!(job.category(), Category::Documents);
    }
    assert_eq!(
        std::fs::read(paths.downloads.join("legacy.pdf")).unwrap(),
        b"preserved user file"
    );
    assert!(!paths.downloads.join("Documents").exists());
}

#[test]
fn invalid_folder_settings_fail_atomically_and_bad_storage_is_reported() {
    let (root, store) = setup();
    let original = store.download_folders().unwrap();
    let mut draft = original.clone();
    draft
        .overrides
        .insert(Category::Documents, root.path().join("valid"));
    draft
        .overrides
        .insert(Category::Music, "relative/folder".into());
    assert!(store.save_download_folders(&draft).is_err());
    assert_eq!(store.download_folders().unwrap(), original);
    assert!(
        store
            .new_download(
                "https://example.test/a.zip",
                None,
                Some(Path::new("relative")),
                "bad".into()
            )
            .is_err()
    );
    let conn = crate::fixture_database::open(store.paths.data.join("queue.sqlite3")).unwrap();
    conn.execute("UPDATE download_folders SET data='{}'", [])
        .unwrap();
    assert!(store.download_folders().is_err());
    assert!(
        store
            .new_download(
                "https://example.test/a.zip",
                None,
                None,
                "bad-settings".into()
            )
            .is_err()
    );
    conn.execute("DELETE FROM download_folders", []).unwrap();
    assert!(store.save_download_folders(&original).is_err());
}

#[cfg(windows)]
#[test]
fn configured_folders_reject_device_network_and_control_character_paths() {
    let (_root, store) = setup();
    for folder in [
        r"C:relative",
        r"\\server\share\files",
        r"\\.\pipe\name",
        r"\\?\C:\Downloads",
        "C:\\Downloads\nInjected",
        "C:\\Downloads\0",
    ] {
        let mut rules = DownloadFolders::default();
        rules.overrides.insert(Category::Other, folder.into());
        assert!(store.save_download_folders(&rules).is_err(), "{folder}");
        assert_eq!(
            store.download_folders().unwrap(),
            DownloadFolders::default()
        );
    }
}

#[test]
fn browser_messages_cannot_supply_a_folder_or_escape_with_a_filename() {
    let (_root, store) = setup();
    let forbidden = serde_json::json!({"action":"add","request_id":"untrusted","url":"https://example.test/file.zip","directory":"C:\\Windows"});
    assert!(!native::handle(&store, &serde_json::to_vec(&forbidden).unwrap()).ok);
    assert!(store.list().unwrap().is_empty());
    let request = serde_json::json!({"action":"add","request_id":"safe","url":"https://example.test/file.zip","filename":"../../elsewhere/file.zip"});
    let reply = native::handle(&store, &serde_json::to_vec(&request).unwrap());
    assert!(reply.ok, "{}", reply.message);
    let saved = store.get(reply.job_id.as_ref().unwrap()).unwrap().unwrap();
    assert_eq!(saved.directory, store.paths.downloads.join("Compressed"));
    assert_eq!(saved.name, ".._.._elsewhere_file.zip");
}

#[test]
fn default_rules_follow_the_portable_root_but_custom_folders_stay_explicit() {
    let (root, _store) = setup();
    let rules: DownloadFolders =
        serde_json::from_str(&serde_json::to_string(&DownloadFolders::default()).unwrap()).unwrap();
    let moved = root.path().join("moved-app/downloads");
    assert_eq!(
        rules.destination(&moved, "new.zip"),
        moved.join("Compressed")
    );
}
