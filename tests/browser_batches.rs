#[path = "../src/desktop/fixture_database.rs"]
mod fixture_database;
use download_engine::{
    browser_batch::{Destination, Link},
    categories::Category,
    model::Status,
    native,
    paths::AppPaths,
    queue::MAIN_QUEUE,
    store::Store,
};
use rusqlite::Connection;
use serde_json::json;
use std::sync::{Arc, Barrier};
use tempfile::TempDir;

fn fixture() -> (TempDir, Store) {
    let dir = TempDir::new().unwrap();
    let store = Store::open(AppPaths {
        data: dir.path().join("data"),
        downloads: dir.path().join("downloads"),
    })
    .unwrap();
    (dir, store)
}
fn id() -> String {
    format!("batch:{}", uuid::Uuid::new_v4())
}
fn links() -> Vec<Link> {
    vec![
        Link {
            url: "https://example.test/a.pdf?token=private".into(),
            filename: Some("a.pdf".into()),
        },
        Link {
            url: "https://example.test/b.zip".into(),
            filename: Some("b.zip".into()),
        },
    ]
}
fn conn(store: &Store) -> Connection {
    crate::fixture_database::open(store.paths.data.join("queue.sqlite3")).unwrap()
}
fn receipts(store: &Store) -> i64 {
    conn(store)
        .query_row("SELECT count(*) FROM browser_batches", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn options_and_batch_use_saved_queues_and_folder_rules() {
    let (_dir, store) = fixture();
    let queue = store.create_queue("Read later").unwrap();
    let mut folders = store.download_folders().unwrap();
    let documents = store.paths.downloads.join("My documents");
    folders
        .overrides
        .insert(Category::Documents, documents.clone());
    store.save_download_folders(&folders).unwrap();
    let reply = native::handle(&store, br#"{"action":"batch_options"}"#);
    assert!(reply.ok);
    let options = reply.batch_options.unwrap();
    assert_eq!(options.destinations.len(), 8);
    assert!(
        options
            .queues
            .iter()
            .any(|q| q.id == queue.id && q.name == "Read later" && !q.running)
    );
    assert!(
        options
            .destinations
            .iter()
            .any(|f| f.path.as_deref() == documents.to_str())
    );
    let saved = store
        .enqueue_browser_batch(&id(), &links(), &queue.id, Destination::Automatic)
        .unwrap();
    let jobs = store.queue_jobs(&queue.id).unwrap();
    assert_eq!(jobs.iter().map(|j| j.id.clone()).collect::<Vec<_>>(), saved);
    assert!(jobs.iter().all(|j| j.status == Status::Queued));
    assert_eq!(jobs[0].directory, documents);
    assert_eq!(jobs[1].directory, store.paths.downloads.join("Compressed"));
    assert!(store.queue_jobs(MAIN_QUEUE).unwrap().is_empty());
    let explicit = store
        .enqueue_browser_batch(&id(), &links(), MAIN_QUEUE, Destination::Documents)
        .unwrap();
    for job in explicit {
        assert_eq!(store.get(&job).unwrap().unwrap().directory, documents);
    }
    let root = store
        .enqueue_browser_batch(&id(), &links(), MAIN_QUEUE, Destination::Downloads)
        .unwrap();
    for job in root {
        assert_eq!(
            store.get(&job).unwrap().unwrap().directory,
            store.paths.downloads
        );
    }
}

#[test]
fn batch_retry_survives_restart_folder_changes_and_removed_history() {
    let (_dir, store) = fixture();
    let token = id();
    let queue = store.create_queue("Later").unwrap();
    let saved = store
        .enqueue_browser_batch(&token, &links(), &queue.id, Destination::Automatic)
        .unwrap();
    let paths = store.paths.clone();
    drop(store);
    let store = Store::open(paths).unwrap();
    let mut rules = store.download_folders().unwrap();
    rules.organize = false;
    store.save_download_folders(&rules).unwrap();
    assert_eq!(
        store
            .enqueue_browser_batch(&token, &links(), &queue.id, Destination::Automatic)
            .unwrap(),
        saved
    );
    assert!(
        store
            .get(&saved[0])
            .unwrap()
            .unwrap()
            .directory
            .ends_with("Documents")
    );
    for job in &saved {
        store.remove(job).unwrap();
    }
    store.delete_queue(&queue.id).unwrap();
    assert_eq!(
        store
            .enqueue_browser_batch(&token, &links(), &queue.id, Destination::Automatic)
            .unwrap(),
        saved
    );
    assert!(store.list().unwrap().is_empty());
    let digest: Vec<u8> = conn(&store)
        .query_row("SELECT fingerprint FROM browser_batches", [], |r| r.get(0))
        .unwrap();
    assert_eq!(digest.len(), 32);
    assert_eq!(receipts(&store), 1);
}

#[test]
fn changed_batch_intent_is_rejected_without_changing_original_jobs() {
    let (_dir, store) = fixture();
    let token = id();
    let saved = store
        .enqueue_browser_batch(&token, &links(), MAIN_QUEUE, Destination::Automatic)
        .unwrap();
    let mut changed = links();
    changed[0].url.push_str("-changed");
    assert!(
        store
            .enqueue_browser_batch(&token, &changed, MAIN_QUEUE, Destination::Automatic)
            .is_err()
    );
    assert!(
        store
            .enqueue_browser_batch(&token, &links(), MAIN_QUEUE, Destination::Downloads)
            .is_err()
    );
    assert!(
        store
            .enqueue_browser_batch(&token, &links(), "different", Destination::Automatic)
            .is_err()
    );
    changed = links();
    changed[0].filename = Some("different.pdf".into());
    assert!(
        store
            .enqueue_browser_batch(&token, &changed, MAIN_QUEUE, Destination::Automatic)
            .is_err()
    );
    assert_eq!(store.list().unwrap().len(), saved.len());
    assert_eq!(receipts(&store), 1);
}

#[test]
fn invalid_batch_never_partially_enqueues_and_protocol_rejects_extra_authority() {
    let (_dir, store) = fixture();
    for invalid in [
        "javascript:alert(1)",
        "file:///C:/secret",
        "https://user:secret@example.test/file",
    ] {
        let mut input = links();
        input[1].url = invalid.into();
        assert!(
            store
                .enqueue_browser_batch(&id(), &input, MAIN_QUEUE, Destination::Automatic)
                .is_err()
        );
    }
    assert!(
        store
            .enqueue_browser_batch(&id(), &[], MAIN_QUEUE, Destination::Automatic)
            .is_err()
    );
    assert!(
        store
            .enqueue_browser_batch(
                &id(),
                &vec![links()[0].clone(); 1001],
                MAIN_QUEUE,
                Destination::Automatic
            )
            .is_err()
    );
    assert!(
        store
            .enqueue_browser_batch(&id(), &links(), "missing", Destination::Automatic)
            .is_err()
    );
    assert!(
        store
            .enqueue_browser_batch(
                "batch:not-uuid",
                &links(),
                MAIN_QUEUE,
                Destination::Automatic
            )
            .is_err()
    );
    let mut long = links();
    long[0].filename = Some("a".repeat(1025));
    assert!(
        store
            .enqueue_browser_batch(&id(), &long, MAIN_QUEUE, Destination::Automatic)
            .is_err()
    );
    for extra in [
        json!({"directory":"C:\\Windows"}),
        json!({"headers":{"Cookie":"secret"}}),
    ] {
        let mut request = json!({"action":"add_batch","request_id":id(),"links":links(),"queue_id":MAIN_QUEUE,"destination":"automatic"});
        for (k, v) in extra.as_object().unwrap() {
            request[k] = v.clone();
        }
        assert!(!native::handle(&store, &serde_json::to_vec(&request).unwrap()).ok);
    }
    let req =
        json!({"action":"add","request_id":format!("{}:0",id()),"url":"https://example.test/a"});
    assert!(!native::handle(&store, &serde_json::to_vec(&req).unwrap()).ok);
    assert!(store.list().unwrap().is_empty());
    assert_eq!(receipts(&store), 0);
}

#[test]
fn browser_batch_deduplicates_urls_and_neutralizes_names() {
    let (_dir, store) = fixture();
    let mut input = links();
    input[0].filename = Some("../CON.pdf".into());
    let mut duplicate = input[0].clone();
    duplicate.url.push_str("#fragment");
    input.push(duplicate);
    let saved = store
        .enqueue_browser_batch(&id(), &input, MAIN_QUEUE, Destination::Downloads)
        .unwrap();
    assert_eq!(saved.len(), 2);
    let job = store.get(&saved[0]).unwrap().unwrap();
    assert!(!job.name.contains('/') && !job.name.contains('\\'));
    assert_eq!(job.directory, store.paths.downloads);
}

#[test]
fn job_or_receipt_failure_rolls_back_the_whole_batch() {
    let (_dir, store) = fixture();
    let db = conn(&store);
    db.execute_batch("CREATE TRIGGER fail_job BEFORE INSERT ON jobs WHEN json_extract(NEW.data,'$.name')='b.zip' BEGIN SELECT RAISE(FAIL,'fixture disk failure'); END;").unwrap();
    assert!(
        store
            .enqueue_browser_batch(&id(), &links(), MAIN_QUEUE, Destination::Automatic)
            .is_err()
    );
    assert!(store.list().unwrap().is_empty());
    assert_eq!(receipts(&store), 0);
    db.execute_batch("DROP TRIGGER fail_job; CREATE TRIGGER fail_receipt BEFORE INSERT ON browser_batches BEGIN SELECT RAISE(FAIL,'fixture receipt failure'); END;").unwrap();
    assert!(
        store
            .enqueue_browser_batch(&id(), &links(), MAIN_QUEUE, Destination::Automatic)
            .is_err()
    );
    assert!(store.list().unwrap().is_empty());
    assert_eq!(receipts(&store), 0);
    let memberships: i64 = db
        .query_row("SELECT count(*) FROM job_queues", [], |r| r.get(0))
        .unwrap();
    assert_eq!(memberships, 0);
}

#[test]
fn simultaneous_retries_return_one_complete_receipt() {
    let (_dir, store) = fixture();
    let token = id();
    let barrier = Arc::new(Barrier::new(6));
    let tasks: Vec<_> = (0..6)
        .map(|_| {
            let s = store.clone();
            let t = token.clone();
            let b = barrier.clone();
            std::thread::spawn(move || {
                b.wait();
                s.enqueue_browser_batch(&t, &links(), MAIN_QUEUE, Destination::Automatic)
                    .unwrap()
            })
        })
        .collect();
    let saved: Vec<_> = tasks.into_iter().map(|t| t.join().unwrap()).collect();
    assert!(saved.iter().all(|ids| ids == &saved[0]));
    assert_eq!(store.list().unwrap().len(), 2);
    assert_eq!(receipts(&store), 1);
}

#[test]
fn framed_native_reply_acknowledges_all_jobs_and_replays_it() {
    let (_dir, store) = fixture();
    let request=serde_json::to_vec(&json!({"action":"add_batch","request_id":id(),"links":links(),"queue_id":MAIN_QUEUE,"destination":"downloads"})).unwrap();
    let reply = native::handle(&store, &request);
    assert!(reply.ok);
    assert_eq!(reply.job_ids.len(), 2);
    let mut bytes = Vec::new();
    native::write_frame(&mut bytes, &reply).unwrap();
    let decoded: native::Reply =
        serde_json::from_slice(&native::read_frame(&mut bytes.as_slice()).unwrap().unwrap())
            .unwrap();
    assert_eq!(decoded.job_ids, native::handle(&store, &request).job_ids);
    assert!(
        native::handle(&store, br#"{"action":"ping"}"#)
            .capabilities
            .contains(&"page_links_batch_v1".into())
    );
}
