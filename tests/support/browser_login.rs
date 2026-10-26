use super::*;
use download_engine::auth;
use serde_json::{Value, json};
use uuid::Uuid;

const SECRET: &str = "SYNTHETIC-BROWSER-LOGIN";
fn cookie() -> Value {
    json!({"name":"session","value":SECRET,"domain":"127.0.0.1","path":"/","host_only":true,"secure":false,"expires":null})
}
fn request(url: &str, cookies: Vec<Value>) -> Value {
    json!({"action":"add_browser_login","request_id":format!("login:{}",Uuid::new_v4()),"url":url,"cookies":cookies})
}
fn handoff(store: &Store, request: &Value) -> native::Reply {
    native::handle(store, &serde_json::to_vec(request).unwrap())
}
fn secured(store: &Store, url: &str, cookies: Vec<Value>) -> Job {
    let reply = handoff(store, &request(url, cookies));
    assert!(reply.ok, "{}", reply.message);
    store.get(&reply.job_id.unwrap()).unwrap().unwrap()
}
fn count(store: &Store, table: &str) -> i64 {
    rusqlite::Connection::open(store.paths.data.join("queue.sqlite3"))
        .unwrap()
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
#[test]
fn browser_login_rejects_wrong_scope_injection_expiry_and_old_protocol_without_echoing_values() {
    let (_temp, store) = setup();
    for (key, value) in [
        ("value", json!("SYNTHETIC-BROWSER-LOGIN\r\nInjected: 1")),
        ("name", json!("SYNTHETIC-BROWSER-LOGIN;bad")),
        ("domain", json!("other.test")),
        ("path", json!("/other")),
        ("secure", json!(true)),
        ("expires", json!(1)),
        ("value", json!("x".repeat(16385))),
    ] {
        let mut value_cookie = cookie();
        value_cookie[key] = value;
        let reply = handoff(
            &store,
            &request("http://127.0.0.1/file", vec![value_cookie]),
        );
        assert!(!reply.ok);
        assert!(!reply.message.contains(SECRET));
    }
    let mut malformed = request("http://127.0.0.1/file", vec![cookie()]);
    malformed["cookies"][0]["secure"] = json!(SECRET);
    assert_eq!(
        handoff(&store, &malformed).message,
        "Invalid native request"
    );
    let mut insecure = request("http://remote.test/file", vec![cookie()]);
    assert!(!handoff(&store, &insecure).ok);
    insecure["action"] = json!("add");
    assert!(!handoff(&store, &insecure).ok);
    assert_eq!(count(&store, "jobs"), 0);
    assert_eq!(count(&store, "browser_sessions"), 0);
}
#[test]
fn browser_login_is_encrypted_atomic_and_idempotent_even_after_removal() {
    let (temp, store) = setup();
    let req = request("http://127.0.0.1/file", vec![cookie()]);
    let reply = handoff(&store, &req);
    assert!(reply.ok, "{}", reply.message);
    let id = reply.job_id.unwrap();
    assert_eq!(handoff(&store, &req).job_id.as_ref(), Some(&id));
    assert_eq!(count(&store, "browser_sessions"), 1);
    let job = store.get(&id).unwrap().unwrap();
    assert_eq!(
        job.authentication.as_ref().unwrap().kind,
        auth::Kind::Browser
    );
    assert!(!serde_json::to_string(&job).unwrap().contains(SECRET));
    for file in std::fs::read_dir(temp.path().join("data")).unwrap() {
        let file = file.unwrap().path();
        if file.is_file() {
            assert!(
                !std::fs::read(file)
                    .unwrap()
                    .windows(SECRET.len())
                    .any(|b| b == SECRET.as_bytes())
            );
        }
    }
    let typed: native::Request = serde_json::from_value(req.clone()).unwrap();
    assert!(!format!("{typed:?}").contains(SECRET));
    store.remove(&id).unwrap();
    assert_eq!(count(&store, "browser_sessions"), 0);
    assert_eq!(handoff(&store, &req).job_id.as_ref(), Some(&id));
    assert_eq!(count(&store, "jobs"), 0);
    let mut different = req.clone();
    different["url"] = json!("http://127.0.0.1/other");
    assert!(!handoff(&store, &different).ok);
}
#[test]
fn browser_login_rolls_back_ciphertext_and_receipt_if_queue_insert_fails() {
    let (_temp, store) = setup();
    let conn = rusqlite::Connection::open(store.paths.data.join("queue.sqlite3")).unwrap();
    conn.execute_batch("CREATE TRIGGER reject_fixture BEFORE INSERT ON jobs BEGIN SELECT RAISE(FAIL,'fixture disk failure'); END;").unwrap();
    assert!(!handoff(&store, &request("http://127.0.0.1/file", vec![cookie()])).ok);
    assert_eq!(count(&store, "browser_sessions"), 0);
    assert_eq!(count(&store, "browser_login_receipts"), 0);
}
#[test]
fn browser_login_concurrent_duplicate_hosts_commit_only_one_session() {
    let (_temp, store) = setup();
    let req = request("http://127.0.0.1/file", vec![cookie()]);
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let store = store.clone();
            let req = req.clone();
            std::thread::spawn(move || handoff(&store, &req))
        })
        .collect();
    let ids: Vec<_> = threads
        .into_iter()
        .map(|t| {
            let r = t.join().unwrap();
            assert!(r.ok, "{}", r.message);
            r.job_id.unwrap()
        })
        .collect();
    assert!(ids.iter().all(|id| id == &ids[0]));
    assert_eq!(count(&store, "jobs"), 1);
    assert_eq!(count(&store, "browser_sessions"), 1);
}
#[tokio::test]
async fn browser_login_downloads_ranges_and_drops_path_cookie_on_same_origin_redirect() {
    let fixture = Fixture::new(8 * 1024 * 1024, 16, false).await;
    let (_temp, store) = setup();
    let url = fixture.url.replace("fixture.bin", "redirect");
    let mut scoped = cookie();
    scoped["name"] = json!("path_only");
    scoped["path"] = json!("/redirect");
    let job = secured(&store, &url, vec![cookie(), scoped]);
    let done = run(&store, job).await;
    assert_eq!(done.status, Status::Complete, "{:?}", done.error);
    assert_eq!(
        std::fs::read(done.saved_path.unwrap()).unwrap(),
        *fixture.data
    );
    assert!(done.authentication.is_none());
    assert_eq!(count(&store, "browser_sessions"), 0);
    let requests = fixture.requests.lock().unwrap();
    assert!(
        requests
            .iter()
            .any(|r| r.starts_with("GET /redirect ") && r.contains("path_only="))
    );
    assert!(
        requests
            .iter()
            .filter(|r| r.starts_with("GET /fixture.bin "))
            .all(|r| !r.contains("path_only=") && r.contains(SECRET))
    );
}
#[tokio::test]
async fn browser_login_cross_origin_redirect_never_contacts_destination() {
    let source = Fixture::new(128, 16, false).await;
    let destination = Fixture::new(128, 0, false).await;
    let port = reqwest::Url::parse(&destination.url)
        .unwrap()
        .port()
        .unwrap();
    let url = source
        .url
        .replace("fixture.bin", &format!("cross-origin/{port}"));
    let (_temp, store) = setup();
    let failed = run(&store, secured(&store, &url, vec![cookie()])).await;
    assert_eq!(failed.status, Status::Failed);
    assert!(failed.error.unwrap().contains("another server"));
    assert!(destination.requests.lock().unwrap().is_empty());
    assert_eq!(count(&store, "browser_sessions"), 1);
    store.remove(&failed.id).unwrap();
}
#[tokio::test]
async fn browser_login_rejection_and_html_page_fail_without_publishing_or_retrying() {
    for mode in [8, 16] {
        let fixture = Fixture::new(128, mode, false).await;
        let (_temp, store) = setup();
        let mut wrong = cookie();
        wrong["value"] = json!("wrong-session");
        let failed = run(&store, secured(&store, &fixture.url, vec![wrong])).await;
        assert_eq!(failed.status, Status::Failed);
        assert!(failed.saved_path.is_none());
        assert!(failed.retry.next_at.is_none());
        assert!(failed.error.as_ref().unwrap().contains(if mode == 8 {
            "web page"
        } else {
            "Browser login expired"
        }));
        assert!(!failed.error.unwrap().contains("wrong-session"));
        store.remove(&failed.id).unwrap();
    }
}
#[tokio::test]
async fn browser_login_tampered_ciphertext_fails_before_any_network_request() {
    let fixture = Fixture::new(128, 16, false).await;
    let (_temp, store) = setup();
    let job = secured(&store, &fixture.url, vec![cookie()]);
    let conn = rusqlite::Connection::open(store.paths.data.join("queue.sqlite3")).unwrap();
    conn.execute("UPDATE browser_sessions SET sealed=zeroblob(400)", [])
        .unwrap();
    let failed = run(&store, job).await;
    assert_eq!(failed.status, Status::Failed);
    assert!(fixture.requests.lock().unwrap().is_empty());
    store.remove(&failed.id).unwrap();
}
#[tokio::test]
async fn browser_login_survives_pause_and_database_reopen_then_cleans_up() {
    let fixture = Fixture::pausable_range(8 * 1024 * 1024, 16).await;
    let (_temp, store) = setup();
    let job = secured(&store, &fixture.url, vec![cookie()]);
    let id = job.id.clone();
    let cancel = CancellationToken::new();
    let task = tokio::spawn(engine::run(
        store.clone(),
        job,
        cancel.clone(),
        download_engine::bandwidth::Limiter::new(&store.bandwidth_limits().unwrap()).unwrap(),
    ));
    until_stalled(&store, async {
        loop {
            let part = store.paths.parts(&id).unwrap().join("segment-0.part");
            if std::fs::metadata(part).is_ok_and(|m| m.len() >= 128 * 1024) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    cancel.cancel();
    task.await.unwrap().unwrap();
    let paused = store.get(&id).unwrap().unwrap();
    assert_eq!(paused.status, Status::Paused);
    assert_eq!(count(&store, "browser_sessions"), 1);
    let paths = store.paths.clone();
    drop(store);
    let reopened = Store::open(paths).unwrap();
    fixture.release_body();
    let done = run(&reopened, paused).await;
    assert_eq!(done.status, Status::Complete, "{:?}", done.error);
    assert_eq!(
        std::fs::read(done.saved_path.unwrap()).unwrap(),
        *fixture.data
    );
    assert_eq!(count(&reopened, "browser_sessions"), 0);
}

#[tokio::test]
async fn browser_login_empty_file_retries_get_after_html_range_error() {
    let fixture = Fixture::new(0, 16, false).await;
    let (_temp, store) = setup();
    let done = run(&store, secured(&store, &fixture.url, vec![cookie()])).await;
    assert_eq!(done.status, Status::Complete, "{:?}", done.error);
    assert!(std::fs::read(done.saved_path.unwrap()).unwrap().is_empty());
    assert_eq!(count(&store, "browser_sessions"), 0);
}
#[tokio::test]
async fn browser_login_transient_html_error_retains_session_and_retry_policy() {
    let fixture = Fixture::new(1024, 17, false).await;
    let (_temp, store) = setup();
    let failed = run(&store, secured(&store, &fixture.url, vec![cookie()])).await;
    assert_eq!(failed.retry.attempts, 1, "{:?}", failed.error);
    assert!(failed.retry.next_at.is_some());
    assert_eq!(count(&store, "browser_sessions"), 1);
    fixture.mode.store(16, Ordering::SeqCst);
    let done = run(&store, failed).await;
    assert_eq!(done.status, Status::Complete, "{:?}", done.error);
    assert_eq!(
        std::fs::read(done.saved_path.unwrap()).unwrap(),
        *fixture.data
    );
}
