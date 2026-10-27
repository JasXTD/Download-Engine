use super::*;
use crate::{
    download_file,
    model::{Plan, Segment, Status},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, AtomicUsize, Ordering},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

const TRANSFER: &str = "0123456789abcdef0123456789abcdef20261027123456";
const SHORT: &str = "https://we.tl/t-fixture123";
fn link() -> String {
    format!("https://wetransfer.com/downloads/{TRANSFER}/abcdef123456")
}
fn store(temp: &tempfile::TempDir) -> Store {
    Store::open(crate::paths::AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap()
}

#[test]
fn links_accept_transfer_and_recipient_formats_without_claiming_ordinary_files() {
    for url in [
        SHORT.to_owned(),
        link(),
        format!("https://studio.wetransfer.com/downloads/{TRANSFER}/{TRANSFER}/abc123"),
    ] {
        assert!(is_url(&url));
        assert_eq!(
            canonical_url(&format!("{url}/?utm_source=test#anchor"))
                .unwrap()
                .as_str(),
            url
        );
    }
    for bad in [
        "http://we.tl/t-example",
        "https://we.tl/",
        "https://we.tl/a/b",
        "https://we.tl:444/t-example",
        "https://user:pass@we.tl/t-example",
        "https://wetransfer.com/downloads/a/b",
        "https://wetransfer.com/downloads/",
        "https://we.tl/%2fprivate",
    ] {
        assert!(is_url(bad));
        assert!(crate::model::validate_download_url(bad).is_err(), "{bad}");
    }
    for other in [
        "https://wetransfer.com/help-center",
        "https://cdn.wetransfer.com/file.zip",
        "https://we.tl.attacker.example/t-file",
        "https://wetransfer.com@attacker.example/downloads/a/b",
    ] {
        assert!(!is_url(other), "{other}");
    }
    assert_eq!(
        crate::model::validate_download_url(SHORT).unwrap().as_str(),
        SHORT
    );
}

#[test]
fn storage_addresses_reject_arbitrary_hosts_credentials_downgrades_and_buckets() {
    for good in [
        "https://download.wetransfer.com/archive?token=secret",
        "https://storm-eu-west-1.wetransfer.net/file",
        "https://wt-prod-euwest1-storm.s3.amazonaws.com/file",
        "https://s3.us-east-1.amazonaws.com/wt-prod-useast1-storm/file",
    ] {
        network::validate_download_url(&Url::parse(good).unwrap()).unwrap();
    }
    for bad in [
        "http://download.wetransfer.com/file",
        "https://download.wetransfer.com.attacker.example/file",
        "https://download.wetransfer.com:444/file",
        "https://user:pass@download.wetransfer.com/file",
        "https://127.0.0.1/file",
        "https://s3.amazonaws.com/unrelated-bucket/file",
        "https://unrelated.s3.amazonaws.com/file",
        "https://storm.wetransfer.net/file#secret",
    ] {
        assert!(
            network::validate_download_url(&Url::parse(bad).unwrap()).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn anonymous_session_scopes_cookies_and_tokens_to_its_api_origin() {
    use reqwest::header::{HeaderMap, HeaderValue, SET_COOKIE};
    let mut session = session::Session::new(
        &Url::parse(&link()).unwrap(),
        TRANSFER,
        b"<meta content='csrf-secret' name='csrf-token'>",
    )
    .unwrap();
    let mut headers = HeaderMap::new();
    for value in [
        "session=ok; Path=/; Secure; HttpOnly",
        "foreign=bad; Domain=attacker.example; Path=/",
        "page=bad; Path=/downloads",
        "default_path=bad",
        "deleted=bad; Path=/; Max-Age=0",
    ] {
        headers.append(SET_COOKIE, HeaderValue::from_str(value).unwrap());
    }
    session
        .update(&Url::parse(&link()).unwrap(), &headers)
        .unwrap();
    let url = session.api.join("download").unwrap();
    let client = reqwest::Client::new();
    let request = session
        .apply(&url, client.post(url.clone()))
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(request.headers()["cookie"], "session=ok");
    assert_eq!(request.headers()["x-csrf-token"], "csrf-secret");
    assert!(request.headers()["x-csrf-token"].is_sensitive());
    let mut rotated = HeaderMap::new();
    rotated.append(
        SET_COOKIE,
        HeaderValue::from_static(
            "api_session=ok; Max-Age=60; Expires=Thu, 01 Jan 1970 00:00:00 GMT",
        ),
    );
    session.update(&url, &rotated).unwrap();
    let request = session
        .apply(&url, client.post(url.clone()))
        .unwrap()
        .build()
        .unwrap();
    assert!(
        request.headers()["cookie"]
            .to_str()
            .unwrap()
            .contains("api_session=ok")
    );
    assert!(
        session
            .apply(
                &Url::parse("https://download.wetransfer.com/file").unwrap(),
                client.get("https://download.wetransfer.com/file")
            )
            .is_err()
    );
}

// Each fixture owns its loopback transport; production URL validation still
// runs before the test-only wire substitution. No global endpoint overrides.
struct Fixture {
    network: network::Network,
    mode: Arc<AtomicU8>,
    generation: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<String>>>,
    data: Arc<Vec<u8>>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Fixture {
    async fn new(size: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let mode = Arc::new(AtomicU8::new(0));
        let generation = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let data = Arc::new((0..size).map(|i| (i % 251) as u8).collect::<Vec<_>>());
        let (m, g, r, d) = (
            mode.clone(),
            generation.clone(),
            requests.clone(),
            data.clone(),
        );
        let server = tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    stream = listener.accept() => {
                        let Ok((mut stream, _)) = stream else { break };
                        let (m, g, r, data) = (m.clone(), g.clone(), r.clone(), d.clone());
                        tasks.spawn(async move {
                            let mut raw = Vec::new(); let mut byte = [0];
                            while !raw.ends_with(b"\r\n\r\n") && raw.len() < 32768 {
                                if stream.read_exact(&mut byte).await.is_err() { return; }
                                raw.push(byte[0]);
                            }
                            let headers = String::from_utf8(raw).unwrap();
                            let length: usize = headers.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length: ").map(str::to_owned)).unwrap_or("0".into()).parse().unwrap();
                            let mut body = vec![0; length];
                            if stream.read_exact(&mut body).await.is_err() { return; }
                            r.lock().unwrap().push(format!("{headers}{}", String::from_utf8_lossy(&body)));
                            let path = headers.split_whitespace().nth(1).unwrap();
                            let mode = m.load(Ordering::SeqCst);
                            let mut status = 200;
                            let mut extra = String::new();
                            let payload = if path.starts_with("/t-") {
                                status = 302;
                                extra = format!("Location: {}\r\n", if mode == 8 { "http://127.0.0.1/private".into() } else if mode == 15 { SHORT.into() } else { link() });
                                Vec::new()
                            } else if path.starts_with("/downloads/") {
                                if mode == 5 { status = 410; Vec::new() }
                                else if mode == 16 { vec![b' '; 2 * 1024 * 1024 + 1] }
                                else { extra = "Set-Cookie: session=fixture-secret; Path=/; Secure; HttpOnly\r\nContent-Type: text/html\r\n".into(); b"<meta name='csrf-token' content='fixture-csrf'>".to_vec() }
                            } else if path.ends_with("/prepare-download") {
                                let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
                                assert_eq!(body["security_hash"], "abcdef123456");
                                assert!(headers.contains("session=fixture-secret"));
                                assert!(headers.contains("fixture-csrf"));
                                extra = "Content-Type: application/json\r\nSet-Cookie: session=rotated-secret; Path=/; Secure\r\n".into();
                                if mode == 6 { status = 403; b"{}".to_vec() }
                                else if mode == 17 { status = 429; extra.push_str("Retry-After: 60\r\n"); b"{}".to_vec() }
                                else { serde_json::json!({"state": if mode == 18 { "expired" } else { "downloadable" }, "password_protected": mode == 7, "recommended_filename":"../bundle.zip", "size": 999999999}).to_string().into_bytes() }
                            } else if path.ends_with("/download") {
                                let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
                                assert_eq!(body["intent"], "entire_transfer");
                                assert!(headers.contains("session=rotated-secret"));
                                let generation = g.fetch_add(1, Ordering::SeqCst);
                                let url = if mode == 9 { "https://attacker.example/file".into() } else { format!("https://download.wetransfer.com/file?token={generation}") };
                                serde_json::json!({"direct_link":url}).to_string().into_bytes()
                            } else if path.starts_with("/file?") {
                                assert!(!headers.to_ascii_lowercase().contains("cookie:"));
                                assert!(!headers.to_ascii_lowercase().contains("authorization:"));
                                assert!(!headers.to_ascii_lowercase().contains("x-csrf-token:"));
                                assert!(!headers.to_ascii_lowercase().contains("referer:"));
                                if mode == 3 { status = 403; Vec::new() }
                                else if mode == 10 { status = 302; extra = "Location: http://127.0.0.1/private\r\n".into(); Vec::new() }
                                else if mode == 11 { extra = "Content-Type: text/html\r\n".into(); b"<html>Verification required</html>".to_vec() }
                                else {
                                    extra = "Content-Type: application/zip\r\n".into();
                                    if mode != 2 { extra.push_str(if mode == 4 { "ETag: \"changed\"\r\n" } else { "ETag: \"fixture-v1\"\r\n" }); }
                                    if mode == 14 { extra.push_str("Content-Disposition: attachment; filename=server.zip\r\n"); }
                                    let range = headers.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("range: bytes=").map(str::to_owned));
                                    if let Some(range) = range.filter(|_| mode != 2) {
                                        if data.is_empty() { status = 416; Vec::new() }
                                        else {
                                            let (start, end) = range.split_once('-').unwrap();
                                            let (start, end): (usize, usize) = (start.parse().unwrap(), end.parse().unwrap());
                                            status = 206;
                                            extra.push_str(&format!("Content-Range: bytes {}-{end}/{}\r\n", if mode == 12 { start + 1 } else { start }, data.len()));
                                            data[start..=end].to_vec()
                                        }
                                    } else { data.as_ref().clone() }
                                }
                            } else { panic!("Unexpected request path: {path}") };
                            let head = format!("HTTP/1.1 {status} Test\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n", payload.len());
                            if stream.write_all(head.as_bytes()).await.is_err() { return; }
                            if path.starts_with("/file?") && payload.len() > 1 && matches!(mode, 1 | 13) {
                                let prefix = payload.len().min(65536);
                                let _ = stream.write_all(&payload[..prefix]).await;
                                if mode == 1 { std::future::pending::<()>().await; }
                            } else { let _ = stream.write_all(&payload).await; }
                        });
                    },
                    done = tasks.join_next(), if !tasks.is_empty() => { if let Some(result) = done { result.unwrap(); } },
                }
            }
        });
        let mut network = network::Network::via(crate::proxy::Route::direct(), false).unwrap();
        network.fixture = Some(base);
        Self {
            network,
            mode,
            generation,
            requests,
            data,
            server,
        }
    }
    fn job(&self, store: &Store, url: &str) -> Job {
        let job = store
            .new_download(url, None, None, uuid::Uuid::new_v4().to_string())
            .unwrap();
        store.enqueue(&job).unwrap()
    }
    async fn run(
        &self,
        store: &Store,
        job: &mut Job,
        cancel: &CancellationToken,
    ) -> Result<Option<FinishedFile>> {
        let root = store.paths.parts(&job.id)?;
        let _guards = download_file::prepare_parts(&store.paths.data.join("parts"), &root).await?;
        tokio::fs::create_dir_all(&job.directory).await?;
        let limiter = Limiter::new(&Default::default())?.with_store(store)?;
        transfer_with_network(store, job, &root, cancel, &limiter, self.network.clone()).await
    }
}

#[tokio::test]
async fn whole_transfer_uses_shared_parallel_downloads_and_keeps_session_data_private() {
    let fixture = Fixture::new(9 * 1024 * 1024).await;
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    let mut job = fixture.job(&store, SHORT);
    let result = fixture
        .run(&store, &mut job, &CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(std::fs::read(result.path).unwrap(), *fixture.data);
    assert_eq!(job.name, "bundle.zip");
    assert_eq!(job.url, SHORT);
    assert_eq!(job.total, Some(fixture.data.len() as u64));
    assert!(job.plan.as_ref().unwrap().segments.len() >= 2);
    assert_eq!(job.plan.as_ref().unwrap().final_url, link());
    let serialized = serde_json::to_string(&store.get(&job.id).unwrap().unwrap()).unwrap();
    for secret in ["token=", "fixture-secret", "rotated-secret", "fixture-csrf"] {
        assert!(!serialized.contains(secret));
    }
    job.validate_persisted().unwrap();
}

#[tokio::test]
async fn single_empty_and_unresumable_transfers_save_exact_bytes_and_preserve_names() {
    for (size, mode, custom, expected) in [
        (0, 0, false, "bundle.zip"),
        (17, 0, false, "bundle.zip"),
        (29, 2, false, "bundle.zip"),
        (31, 14, false, "server.zip"),
        (33, 14, true, "custom.zip"),
    ] {
        let fixture = Fixture::new(size).await;
        fixture.mode.store(mode, Ordering::SeqCst);
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let mut job = fixture.job(&store, &link());
        if custom {
            job.name = "custom.zip".into();
            job.server_name_pending = false;
        }
        let result = fixture
            .run(&store, &mut job, &CancellationToken::new())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(std::fs::read(result.path).unwrap(), *fixture.data);
        assert_eq!(job.name, expected);
        if mode == 2 || size == 0 {
            assert!(job.plan.is_none());
        }
    }
}

#[tokio::test]
async fn pause_flushes_parts_and_resume_refreshes_the_address_without_repeating_saved_ranges() {
    let fixture = Fixture::new(1024 * 1024).await;
    fixture.mode.store(1, Ordering::SeqCst);
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    let mut job = fixture.job(&store, SHORT);
    let cancel = CancellationToken::new();
    let parts = store.paths.parts(&job.id).unwrap();
    let id = job.id.clone();
    let wait = async {
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                // Wait for observable received progress, then verify that
                // cancellation flushes even bytes still buffered in memory.
                if store
                    .get(&id)
                    .unwrap()
                    .is_some_and(|job| job.downloaded > 0)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        cancel.cancel();
    };
    let (result, ()) = tokio::join!(fixture.run(&store, &mut job, &cancel), wait);
    assert!(result.unwrap().is_none());
    let saved = std::fs::metadata(engine::part_path(&parts, 0))
        .unwrap()
        .len();
    assert!(saved > 0);
    fixture.mode.store(0, Ordering::SeqCst);
    fixture.requests.lock().unwrap().clear();
    let result = fixture
        .run(&store, &mut job, &CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(std::fs::read(result.path).unwrap(), *fixture.data);
    assert_eq!(fixture.generation.load(Ordering::SeqCst), 2);
    assert!(
        fixture
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.starts_with("GET /file?token=1 ")
                && r.contains(&format!("range: bytes={saved}-")))
    );
}

#[tokio::test]
async fn resume_refuses_changed_file_or_transfer_identity_without_overwriting_parts() {
    for changed_transfer in [false, true] {
        let fixture = Fixture::new(100).await;
        if !changed_transfer {
            fixture.mode.store(4, Ordering::SeqCst);
        }
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let mut job = fixture.job(&store, SHORT);
        job.plan = Some(Plan {
            size: 100,
            etag: "\"fixture-v1\"".into(),
            final_url: if changed_transfer {
                link().replace("abcdef123456", "abc123")
            } else {
                link()
            },
            segments: vec![Segment { start: 0, end: 99 }],
        });
        job.total = Some(100);
        job.downloaded = 3;
        job.server_name_pending = false;
        let parts = store.paths.parts(&job.id).unwrap();
        std::fs::create_dir_all(&parts).unwrap();
        std::fs::write(engine::part_path(&parts, 0), [0, 1, 2]).unwrap();
        let error = fixture
            .run(&store, &mut job, &CancellationToken::new())
            .await
            .err()
            .expect("changed transfer must be rejected");
        assert!(
            error.to_string().contains("remote file changed"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read(engine::part_path(&parts, 0)).unwrap(),
            [0, 1, 2]
        );
        assert!(job.saved_path.is_none());
    }
}

#[tokio::test]
async fn failures_never_publish_pages_or_follow_untrusted_redirects() {
    for (mode, message) in [
        (3, "expired"),
        (5, "expired"),
        (6, "verification"),
        (7, "Password"),
        (8, "HTTPS"),
        (9, "unsupported file server"),
        (10, "unsupported file server"),
        (11, "instead of a file"),
        (12, "bounds"),
        (13, "server connection"),
        (15, "redirects"),
        (16, "too large"),
        (17, "429"),
        (18, "not available"),
    ] {
        let fixture = Fixture::new(128 * 1024).await;
        fixture.mode.store(mode, Ordering::SeqCst);
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let mut job = fixture.job(&store, SHORT);
        let error = fixture
            .run(&store, &mut job, &CancellationToken::new())
            .await
            .err()
            .expect("invalid response must be rejected");
        assert!(
            format!("{error:#}").contains(message),
            "mode {mode}: {error:#}"
        );
        assert!(job.saved_path.is_none());
        assert!(
            !fixture
                .requests
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.contains("/private"))
        );
        if matches!(mode, 3 | 13 | 17) {
            crate::retry::failed(&mut job, &error, chrono::Utc::now().timestamp_millis());
            assert!(job.retry.next_at.is_some(), "mode {mode}: {error:#}");
        }
    }
}

#[tokio::test]
async fn cancelled_lookup_makes_no_network_request() {
    let fixture = Fixture::new(16).await;
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    let mut job = fixture.job(&store, SHORT);
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(
        fixture
            .run(&store, &mut job, &cancel)
            .await
            .unwrap()
            .is_none()
    );
    assert!(fixture.requests.lock().unwrap().is_empty());
}

#[test]
fn native_and_batch_entry_points_accept_share_links_and_reject_mixed_state() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    let reply = crate::native::handle_request(
        &store,
        crate::native::Request::Add {
            request_id: "we-native".into(),
            url: SHORT.into(),
            filename: None,
            source_page: None,
        },
    );
    assert!(reply.ok, "{}", reply.message);
    let mut job = store
        .get(reply.job_id.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(job.url, SHORT);
    let ids = store
        .enqueue_browser_batch(
            &format!("batch:{}", uuid::Uuid::new_v4()),
            &[crate::browser_batch::Link {
                url: link(),
                filename: None,
            }],
            crate::queue::MAIN_QUEUE,
            crate::browser_batch::Destination::Downloads,
        )
        .unwrap();
    assert_eq!(ids.len(), 1);
    job.plan = Some(Plan {
        size: 10,
        etag: "\"v1\"".into(),
        final_url: "https://download.wetransfer.com/file?token=old".into(),
        segments: vec![Segment { start: 0, end: 9 }],
    });
    job.total = Some(10);
    assert!(job.validate_persisted().is_err());
    job.plan.as_mut().unwrap().final_url = link();
    job.validate_persisted().unwrap();
    job.status = Status::Complete;
    assert!(
        crate::synchronization::State::begin(&mut job)
            .unwrap_err()
            .to_string()
            .contains("WeTransfer")
    );
}

/// Explicit opt-in only: never fetch a live share from the regular test suite.
#[tokio::test]
#[ignore = "requires an explicitly supplied public link and isolated output directory"]
async fn live_public_transfer() {
    use sha2::{Digest, Sha256};
    let url =
        std::env::var("DOWNLOAD_ENGINE_WETRANSFER_TEST_URL").expect("Supply a public test link");
    let parent = std::path::PathBuf::from(
        std::env::var_os("DOWNLOAD_ENGINE_WETRANSFER_TEST_DIR")
            .expect("Supply an isolated output directory"),
    );
    assert!(parent.is_absolute());
    std::fs::create_dir_all(&parent).unwrap();
    let temp = tempfile::Builder::new()
        .prefix("live-")
        .tempdir_in(&parent)
        .unwrap();
    let store = store(&temp);
    let output = temp.keep();
    println!("Isolated live-check directory: {}", output.display());
    const LIMIT: u64 = 256 * 1024 * 1024;
    store
        .set_quota_limits_at(
            crate::quota::Limits {
                hourly: Some(LIMIT),
                daily: Some(LIMIT),
            },
            chrono::Utc::now().timestamp(),
        )
        .unwrap();
    let source = tokio::time::timeout(
        Duration::from_secs(60),
        network::Network::new(&store, false).unwrap().resolve(&url),
    )
    .await
    .unwrap()
    .unwrap();
    println!("Resolved transfer: {}", source.name);
    // This opt-in diagnostic is also used by agents that must not retrieve
    // web-hosted text documents. Do not turn a resolved .txt into a file fetch.
    assert!(
        !source.name.to_ascii_lowercase().ends_with(".txt"),
        "Choose a non-.txt file for this live diagnostic"
    );
    let response = source.get(Some((0, 0)), None).await.unwrap();
    let size = if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        Some(engine::range(&response).unwrap().2)
    } else {
        response.content_length()
    };
    println!(
        "File response: HTTP {}; size: {:?}",
        response.status().as_u16(),
        size
    );
    assert!(
        size.is_none_or(|n| n <= LIMIT),
        "Live fixture exceeds the test size limit"
    );
    drop(response);
    let job = store
        .new_download(&url, None, None, uuid::Uuid::new_v4().to_string())
        .unwrap();
    let job = store.enqueue(&job).unwrap();
    let id = job.id.clone();
    let cancel = CancellationToken::new();
    let timeout_cancel = cancel.clone();
    let timer = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(180)).await;
        timeout_cancel.cancel();
    });
    let result = engine::run(
        store.clone(),
        job,
        cancel,
        Limiter::new(&Default::default()).unwrap(),
    )
    .await;
    timer.abort();
    result.unwrap();
    let job = store.get(&id).unwrap().unwrap();
    assert_eq!(
        job.status,
        Status::Complete,
        "{}",
        job.error.as_deref().unwrap_or("Transfer did not complete")
    );
    let path = job.saved_path.unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes.len() as u64, job.downloaded);
    let hash = format!("{:x}", Sha256::digest(&bytes));
    assert_eq!(job.integrity.actual.as_deref(), Some(hash.as_str()));
    println!(
        "Downloaded {} bytes to {}; SHA-256 {hash}",
        bytes.len(),
        path.display()
    );
}
