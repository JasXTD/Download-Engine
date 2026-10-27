//! Real loopback HTTP fixtures; no hosting service, browser session or user data.
use download_engine::{
    bandwidth::Limiter,
    engine,
    link_fallback::Selection,
    model::{Job, Status},
    paths::AppPaths,
    scheduler::{Command, Scheduler},
    store::Store,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct Reply {
    body: Arc<Vec<u8>>,
    mime: &'static str,
    status: u16,
    ranges: bool,
    disposition: Option<&'static str>,
    redirect: Option<String>,
    length: bool,
    stall: bool,
    etag: &'static str,
    probe_mime: Option<&'static str>,
    full_redirect: Option<String>,
}
impl Reply {
    fn page(html: &str) -> Self {
        Self {
            body: Arc::new(html.as_bytes().to_vec()),
            mime: "text/html; charset=utf-8",
            status: 200,
            ranges: false,
            disposition: None,
            redirect: None,
            length: true,
            stall: false,
            etag: "\"fixture-v1\"",
            probe_mime: None,
            full_redirect: None,
        }
    }
    fn file(bytes: Vec<u8>) -> Self {
        Self {
            body: Arc::new(bytes),
            mime: "application/octet-stream",
            ranges: true,
            ..Self::page("")
        }
    }
}
struct Server {
    base: String,
    routes: Arc<Mutex<HashMap<String, Reply>>>,
    requests: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let routes = Arc::new(Mutex::new(HashMap::<String, Reply>::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (r, q) = (routes.clone(), requests.clone());
        let task = tokio::spawn(async move {
            let mut handlers = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    result = listener.accept() => {
                        let Ok((mut stream, _)) = result else { break; };
                        let (r, q) = (r.clone(), q.clone());
                        handlers.spawn(async move {
                            let mut raw = Vec::new();
                            loop {
                                let byte = stream.read_u8().await?;
                                raw.push(byte);
                                if raw.ends_with(b"\r\n\r\n") || raw.len() > 16384 { break; }
                            }
                            let request = String::from_utf8(raw).unwrap();
                            let path = request.lines().next().unwrap().split_whitespace().nth(1).unwrap();
                            let reply = r.lock().unwrap().get(path).cloned().unwrap_or(Reply { status: 404, ..Reply::page("") });
                            q.lock().unwrap().push(request.clone());
                            let mut status = reply.status;
                            let mut body = reply.body.as_slice();
                            let mime = if request.to_lowercase().contains("range: bytes=0-0") { reply.probe_mime.unwrap_or(reply.mime) } else { reply.mime };
                            let mut headers = format!("Content-Type: {mime}\r\nConnection: close\r\n");
                            if let Some(location) = &reply.redirect { headers.push_str(&format!("Location: {location}\r\n")); }
                            if let Some(location) = &reply.full_redirect && !request.to_lowercase().contains("range:") {
                                status = 302;
                                headers.push_str(&format!("Location: {location}\r\n"));
                            }
                            if let Some(disposition) = reply.disposition { headers.push_str(&format!("Content-Disposition: {disposition}\r\n")); }
                            if reply.ranges && status == 200 {
                                headers.push_str(&format!("ETag: {}\r\n", reply.etag));
                                if let Some(range) = request.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("range: bytes=").map(str::to_owned)) {
                                    let (start, end) = range.split_once('-').unwrap();
                                    let (start, end) = (start.parse::<usize>().unwrap(), end.parse::<usize>().unwrap());
                                    if end < body.len() {
                                        headers.push_str(&format!("Content-Range: bytes {start}-{end}/{}\r\n", body.len()));
                                        body = &body[start..=end];
                                        status = 206;
                                    } else { status = 416; body = &[]; }
                                }
                            }
                            if reply.length { headers.push_str(&format!("Content-Length: {}\r\n", body.len())); }
                            stream.write_all(format!("HTTP/1.1 {status} Fixture\r\n{headers}\r\n").as_bytes()).await?;
                            if reply.stall { std::future::pending::<()>().await; }
                            stream.write_all(body).await?;
                            stream.shutdown().await
                        });
                    },
                    _ = handlers.join_next(), if !handlers.is_empty() => {},
                }
            }
        });
        Self {
            base,
            routes,
            requests,
            task,
        }
    }
    fn add(&self, path: &str, reply: Reply) {
        self.routes.lock().unwrap().insert(path.into(), reply);
    }
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }
    fn count(&self, path: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.starts_with(&format!("GET {path} HTTP/")))
            .count()
    }
}
fn setup() -> (TempDir, Store) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    (temp, store)
}
fn add(store: &Store, url: &str, name: Option<&str>) -> Job {
    let mut job = Job::new(
        url,
        name,
        store.paths.downloads.clone(),
        uuid::Uuid::new_v4().to_string(),
    )
    .unwrap();
    job.status = Status::Downloading;
    store.enqueue(&job).unwrap()
}
async fn run(store: &Store, job: Job) -> Job {
    run_with_token(store, job, CancellationToken::new()).await
}
async fn run_with_token(store: &Store, mut job: Job, cancel: CancellationToken) -> Job {
    job.status = Status::Downloading;
    store.save(&job).unwrap();
    let id = job.id.clone();
    tokio::time::timeout(
        Duration::from_secs(15),
        engine::run(
            store.clone(),
            job,
            cancel,
            Limiter::new(&store.bandwidth_limits().unwrap()).unwrap(),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    store.get(&id).unwrap().unwrap()
}
async fn finished(store: &Store, id: &str) -> Job {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let job = store.get(id).unwrap().unwrap();
            if job.status == Status::Complete {
                return job;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("selected download did not finish")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_link_uses_normal_engine_and_preserves_original_receipt() {
    let server = Server::new().await;
    server.add(
        "/share",
        Reply::page("<img src='/logo.png'><a download='archive.zip' href='/payload'>Download</a>"),
    );
    let data = (0..9 * 1024 * 1024)
        .map(|n| (n % 251) as u8)
        .collect::<Vec<_>>();
    server.add("/payload", Reply::file(data.clone()));
    let (_temp, store) = setup();
    let original = add(&store, &server.url("/share"), None);
    let job = run(&store, original.clone()).await;
    assert_eq!(job.status, Status::Complete, "{:?}", job.error);
    assert_eq!(job.url, original.url);
    assert_eq!(job.directory, original.directory);
    assert_eq!(job.request_id, original.request_id);
    assert_eq!(job.name, "archive.zip");
    assert_eq!(
        job.link_fallback.as_ref().unwrap().selection,
        Some(Selection::Automatic)
    );
    assert!(job.plan.as_ref().unwrap().segments.len() > 1);
    assert_eq!(
        std::fs::read(job.saved_path.as_ref().unwrap()).unwrap(),
        data
    );
    assert!(job.integrity.actual.is_some());
    assert_eq!(server.count("/share"), 1);
    assert_eq!(server.count("/logo.png"), 0);
    assert_eq!(
        store.enqueue(&original).unwrap().id,
        original.id,
        "original native receipt remains idempotent"
    );
    assert_eq!(store.list().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ambiguous_links_persist_and_only_the_selected_file_is_downloaded() {
    let server = Server::new().await;
    server.add(
        "/share",
        Reply::page("<a href='/one.zip'>One</a><a download href='/two.zip'>Two</a>"),
    );
    server.add("/one.zip", Reply::file(b"wrong file".to_vec()));
    server.add("/two.zip", Reply::file(b"selected file".to_vec()));
    let (_temp, store) = setup();
    let job = run(&store, add(&store, &server.url("/share"), Some("mine.zip"))).await;
    assert_eq!(job.status, Status::Failed);
    assert!(job.retry.next_at.is_none());
    assert!(job.plan.is_none() && job.saved_path.is_none());
    assert_eq!(job.downloaded, 0);
    assert_eq!(server.count("/one.zip") + server.count("/two.zip"), 0);
    let reopened = Store::open(store.paths.clone()).unwrap();
    let job = reopened.get(&job.id).unwrap().unwrap();
    let discovery = job.link_fallback.as_ref().unwrap().id;
    let scheduler = Scheduler::start(reopened.clone()).unwrap();
    scheduler
        .send(Command::SelectDiscoveredFile {
            id: job.id.clone(),
            discovery,
            selection: Selection::File(1),
        })
        .unwrap();
    let saved = finished(&reopened, &job.id).await;
    assert_eq!(saved.name, "mine.zip");
    assert_eq!(saved.url, job.url);
    assert_eq!(
        std::fs::read(saved.saved_path.unwrap()).unwrap(),
        b"selected file"
    );
    assert_eq!(server.count("/share"), 1);
    assert_eq!(server.count("/one.zip"), 0);
    drop(scheduler);
}

#[tokio::test]
async fn single_ordinary_link_and_external_download_link_require_review() {
    let server = Server::new().await;
    let other = Server::new().await;
    let (_temp, store) = setup();
    for html in [
        "<a href='/terms.pdf'>Terms</a>".to_owned(),
        format!("<a download href='{}'>Download</a>", other.url("/file.zip")),
    ] {
        server.add("/share", Reply::page(&html));
        let job = run(&store, add(&store, &server.url("/share"), None)).await;
        assert_eq!(job.status, Status::Failed);
        assert!(job.link_fallback.unwrap().needs_review());
    }
    assert_eq!(server.count("/terms.pdf"), 0);
    assert!(other.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn automatic_redirect_does_not_fetch_another_origin() {
    let server = Server::new().await;
    let other = Server::new().await;
    server.add(
        "/share",
        Reply::page("<a download href='/out'>Download</a>"),
    );
    server.add(
        "/out",
        Reply {
            status: 302,
            redirect: Some(other.url("/file.zip")),
            ..Reply::page("")
        },
    );
    let (_temp, store) = setup();
    let job = run(&store, add(&store, &server.url("/share"), None)).await;
    assert_eq!(job.status, Status::Failed);
    assert!(job.error.unwrap().contains("another site"));
    assert!(job.link_fallback.unwrap().needs_review());
    assert!(other.requests.lock().unwrap().is_empty());
    // A file can redirect only on its full GET after an apparently valid probe.
    server.add(
        "/out",
        Reply {
            etag: "W/\"weak\"",
            full_redirect: Some(other.url("/file.zip")),
            ..Reply::file(b"file".to_vec())
        },
    );
    let job = run(&store, add(&store, &server.url("/share"), None)).await;
    assert_eq!(job.status, Status::Failed);
    assert!(job.link_fallback.unwrap().needs_review());
    assert!(other.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn no_links_and_another_html_page_never_become_file_bytes() {
    let server = Server::new().await;
    server.add("/next", Reply::page("<a download href='/share'>Loop</a>"));
    let (_temp, store) = setup();
    for html in [
        "<script>location='/secret.zip'</script><img src='/logo.zip'>",
        "<a download href='/next'>Download</a>",
    ] {
        server.add("/share", Reply::page(html));
        let job = run(&store, add(&store, &server.url("/share"), None)).await;
        assert_eq!(job.status, Status::Failed);
        assert!(job.link_fallback.unwrap().needs_review());
        assert!(job.saved_path.is_none() && job.plan.is_none());
        assert_eq!(job.downloaded, 0);
    }
    assert_eq!(server.count("/share"), 2, "no recursive crawling");
    assert_eq!(server.count("/secret.zip") + server.count("/logo.zip"), 0);
}

#[tokio::test]
async fn error_pages_keep_authentication_and_retry_behavior() {
    let server = Server::new().await;
    let (_temp, store) = setup();
    for status in [401, 403, 404, 429, 503] {
        server.add(
            "/share",
            Reply {
                status,
                ..Reply::page("<a download href='/file.zip'>Not a successful page</a>")
            },
        );
        let job = run(&store, add(&store, &server.url("/share"), None)).await;
        assert!(job.link_fallback.is_none());
        assert_eq!(job.retry.next_at.is_some(), matches!(status, 429 | 503));
        assert!(job.saved_path.is_none());
    }
    assert_eq!(server.count("/file.zip"), 0);
}

#[tokio::test]
async fn explicit_html_downloads_and_attachments_keep_existing_behavior() {
    let server = Server::new().await;
    let html = "<a download href='/file.zip'>This HTML is itself the requested file</a>";
    server.add("/document.html", Reply::page(html));
    server.add(
        "/attachment",
        Reply {
            disposition: Some("attachment; filename=report.html"),
            ..Reply::page(html)
        },
    );
    server.add("/custom", Reply::page(html));
    let (_temp, store) = setup();
    for (path, name) in [
        ("/document.html", Some("document.html")),
        ("/attachment", None),
        ("/custom", Some("mine.html")),
    ] {
        let job = run(&store, add(&store, &server.url(path), name)).await;
        assert_eq!(job.status, Status::Complete, "{:?}", job.error);
        assert!(job.link_fallback.is_none());
        assert_eq!(
            std::fs::read(job.saved_path.unwrap()).unwrap(),
            html.as_bytes()
        );
    }
    assert_eq!(server.count("/file.zip"), 0);
}

#[tokio::test]
async fn partial_page_probe_is_replaced_by_a_bounded_full_page_read() {
    let server = Server::new().await;
    server.add(
        "/share.html",
        Reply {
            ranges: true,
            ..Reply::page("<a download href='/file.zip'>Download</a>")
        },
    );
    server.add("/file.zip", Reply::file(b"file".to_vec()));
    let (_temp, store) = setup();
    let job = run(&store, add(&store, &server.url("/share.html"), None)).await;
    assert_eq!(job.status, Status::Complete, "{:?}", job.error);
    assert_eq!(server.count("/share.html"), 2);
    assert_eq!(std::fs::read(job.saved_path.unwrap()).unwrap(), b"file");
}

#[tokio::test]
async fn full_get_html_after_a_file_typed_probe_also_uses_fallback() {
    let server = Server::new().await;
    server.add(
        "/share",
        Reply {
            ranges: true,
            etag: "W/\"weak\"",
            probe_mime: Some("application/octet-stream"),
            ..Reply::page("<a download href='/file.zip'>Download</a>")
        },
    );
    server.add("/file.zip", Reply::file(b"actual file".to_vec()));
    let (_temp, store) = setup();
    let job = run(&store, add(&store, &server.url("/share"), None)).await;
    assert_eq!(job.status, Status::Complete, "{:?}", job.error);
    assert!(job.link_fallback.is_some());
    assert_eq!(server.count("/share"), 2);
    assert_eq!(
        std::fs::read(job.saved_path.unwrap()).unwrap(),
        b"actual file"
    );
}

#[tokio::test]
async fn oversized_page_is_rejected_with_and_without_content_length() {
    let server = Server::new().await;
    let (_temp, store) = setup();
    for length in [true, false] {
        server.add(
            "/share",
            Reply {
                body: Arc::new(vec![b'x'; 2 * 1024 * 1024 + 1]),
                length,
                ..Reply::page("")
            },
        );
        let job = run(&store, add(&store, &server.url("/share"), None)).await;
        assert_eq!(job.status, Status::Failed);
        assert!(job.error.unwrap().contains("2 MiB limit"));
        assert_eq!(job.downloaded, 0);
        assert!(job.saved_path.is_none() && job.plan.is_none());
    }
}

#[tokio::test]
async fn cancellation_during_page_read_stops_discovery_without_enqueuing_files() {
    let server = Server::new().await;
    server.add(
        "/share",
        Reply {
            stall: true,
            ..Reply::page("<a download href='/file.zip'>Download</a>")
        },
    );
    let (_temp, store) = setup();
    let cancel = CancellationToken::new();
    let job = add(&store, &server.url("/share"), None);
    let running = run_with_token(&store, job, cancel.clone());
    let stop = async {
        tokio::time::timeout(Duration::from_secs(5), async {
            while server.count("/share") == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        cancel.cancel();
    };
    let (job, _) = tokio::join!(running, stop);
    assert_eq!(job.status, Status::Paused);
    assert!(job.link_fallback.is_none() && job.saved_path.is_none());
    assert_eq!(store.list().unwrap().len(), 1);
    assert_eq!(server.count("/file.zip"), 0);
}

#[tokio::test]
async fn resume_uses_selected_file_and_rejects_a_changed_validator() {
    use download_engine::model::{Plan, Segment};
    let server = Server::new().await;
    server.add("/share", Reply::page("<a href='/file.zip'>File</a>"));
    let data = (0..256 * 1024).map(|n| (n % 251) as u8).collect::<Vec<_>>();
    let (_temp, store) = setup();
    for changed in [false, true] {
        let mut job = run(&store, add(&store, &server.url("/share"), None)).await;
        job.link_fallback.as_mut().unwrap().selection = Some(Selection::File(0));
        job.status = Status::Paused;
        job.server_name_pending = false;
        job.name = "file.zip".into();
        job.downloaded = 65536;
        job.total = Some(data.len() as u64);
        job.plan = Some(Plan {
            size: data.len() as u64,
            etag: "\"fixture-v1\"".into(),
            final_url: server.url("/file.zip"),
            segments: vec![Segment {
                start: 0,
                end: data.len() as u64 - 1,
            }],
        });
        let parts = store.paths.parts(&job.id).unwrap();
        std::fs::create_dir_all(&parts).unwrap();
        let part = parts.join("segment-0.part");
        std::fs::write(&part, &data[..65536]).unwrap();
        server.add(
            "/file.zip",
            Reply {
                etag: if changed {
                    "\"fixture-v2\""
                } else {
                    "\"fixture-v1\""
                },
                ..Reply::file(data.clone())
            },
        );
        let saved = run(&store, job).await;
        if changed {
            assert_eq!(saved.status, Status::Failed);
            assert!(saved.error.unwrap().contains("remote file changed"));
            assert_eq!(std::fs::read(&part).unwrap(), data[..65536]);
            assert!(saved.saved_path.is_none());
        } else {
            assert_eq!(saved.status, Status::Complete, "{:?}", saved.error);
            assert_eq!(std::fs::read(saved.saved_path.unwrap()).unwrap(), data);
            assert!(
                server
                    .requests
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|request| request.to_lowercase().contains("range: bytes=65536-262143"))
            );
        }
    }
    assert_eq!(
        server.count("/share"),
        2,
        "resume does not reselect a different page link"
    );
}

#[tokio::test]
async fn browser_capture_html_mismatch_does_not_trigger_fallback() {
    let server = Server::new().await;
    server.add(
        "/file",
        Reply::page("<a download href='/other.zip'>Other</a>"),
    );
    let (_temp, store) = setup();
    let mut job = add(&store, &server.url("/file"), Some("captured.zip"));
    job.expected_file =
        Some(download_engine::capture::ExpectedFile::browser(42, "application/zip").unwrap());
    let job = run(&store, job).await;
    assert_eq!(job.status, Status::Failed);
    assert!(job.error.unwrap().contains("different file type"));
    assert!(job.link_fallback.is_none() && job.saved_path.is_none());
    assert_eq!(server.count("/other.zip"), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_save_page_and_restart_use_the_original_url() {
    let server = Server::new().await;
    let html = "<p>This page has no download links</p>";
    server.add("/page", Reply::page(html));
    server.add("/restart", Reply::page(html));
    server.add("/file.zip", Reply::file(b"new file".to_vec()));
    let (_temp, store) = setup();
    let page = run(&store, add(&store, &server.url("/page"), None)).await;
    let restart = run(&store, add(&store, &server.url("/restart"), None)).await;
    let old_id = restart.link_fallback.as_ref().unwrap().id;
    server.add(
        "/restart",
        Reply::page("<a download href='/file.zip'>File</a>"),
    );
    let scheduler = Scheduler::start(store.clone()).unwrap();
    scheduler
        .send(Command::SelectDiscoveredFile {
            id: page.id.clone(),
            discovery: page.link_fallback.unwrap().id,
            selection: Selection::Page,
        })
        .unwrap();
    scheduler
        .send(Command::Restart(restart.id.clone()))
        .unwrap();
    let saved_page = finished(&store, &page.id).await;
    assert_eq!(saved_page.name, "page.html");
    assert_eq!(
        std::fs::read(saved_page.saved_path.unwrap()).unwrap(),
        html.as_bytes()
    );
    let restarted = finished(&store, &restart.id).await;
    assert_ne!(restarted.link_fallback.as_ref().unwrap().id, old_id);
    assert_eq!(restarted.url, restart.url);
    assert_eq!(
        std::fs::read(restarted.saved_path.unwrap()).unwrap(),
        b"new file"
    );
    assert_eq!(server.count("/restart"), 2);
    drop(scheduler);
}

/// Opt-in real-host check. It downloads only Mozilla's public PDF fixture, never
/// executes it, and uses a fresh profile below the supplied artifacts directory.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires explicit live GitHub test authorization and an artifacts directory"]
async fn live_github_file_page_fallback() {
    use sha2::{Digest, Sha256};
    const PAGE: &str = "https://github.com/mozilla/pdf.js/blob/master/test/pdfs/tracemonkey.pdf";
    const RAW: &str =
        "https://raw.githubusercontent.com/mozilla/pdf.js/master/test/pdfs/tracemonkey.pdf";
    let root = std::path::PathBuf::from(
        std::env::var_os("DOWNLOAD_ENGINE_LIVE_TEST_ROOT")
            .expect("Set DOWNLOAD_ENGINE_LIVE_TEST_ROOT to an isolated artifacts directory"),
    );
    assert!(root.is_absolute());
    std::fs::create_dir_all(&root).unwrap();
    let profile = tempfile::Builder::new()
        .prefix("github-fallback-")
        .tempdir_in(&root)
        .unwrap()
        .keep();
    let store = Store::open(AppPaths {
        data: profile.join("data"),
        downloads: profile.join("downloads"),
    })
    .unwrap();
    let job = add(&store, PAGE, None);
    let id = job.id.clone();
    tokio::time::timeout(
        Duration::from_secs(90),
        engine::run(
            store.clone(),
            job,
            CancellationToken::new(),
            Limiter::new(&store.bandwidth_limits().unwrap()).unwrap(),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let initial = store.get(&id).unwrap().unwrap();
    std::fs::write(
        profile.join("initial-job.json"),
        serde_json::to_vec_pretty(&initial).unwrap(),
    )
    .unwrap();
    println!("Live profile: {}", profile.display());
    let discovery = initial
        .link_fallback
        .as_ref()
        .expect("GitHub HTML must exercise the generic fallback");
    println!(
        "Initial status: {:?}; candidates: {}; error: {:?}",
        initial.status,
        discovery.files.len(),
        initial.error
    );
    let candidate = discovery
        .files
        .iter()
        .position(|file| {
            let url = reqwest::Url::parse(&file.url).unwrap();
            url.scheme() == "https"
                && url.host_str() == Some("github.com")
                && matches!(
                    url.path(),
                    "/mozilla/pdf.js/raw/master/test/pdfs/tracemonkey.pdf"
                        | "/mozilla/pdf.js/raw/refs/heads/master/test/pdfs/tracemonkey.pdf"
                )
                && url.query().is_none()
        })
        .expect("The verified public PDF should appear in the fallback results");
    assert!(
        discovery.needs_review(),
        "Cross-origin GitHub raw downloads should be reviewed"
    );
    let scheduler = Scheduler::start(store.clone()).unwrap();
    scheduler
        .send(Command::SelectDiscoveredFile {
            id: id.clone(),
            discovery: discovery.id,
            selection: Selection::File(candidate),
        })
        .unwrap();
    let saved = tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            let job = store.get(&id).unwrap().unwrap();
            if matches!(job.status, Status::Complete | Status::Failed)
                && !job.link_fallback.as_ref().unwrap().needs_review()
            {
                break job;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(saved.status, Status::Complete, "{:?}", saved.error);
    assert_eq!(saved.name, "tracemonkey.pdf");
    assert_eq!(saved.url, PAGE);
    let bytes = std::fs::read(saved.saved_path.as_ref().unwrap()).unwrap();
    assert!(bytes.starts_with(b"%PDF-") && bytes.len() < 2 * 1024 * 1024);
    // An independent, bounded read of the same official PDF checks exact bytes.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();
    let mut response = client.get(RAW).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    // GitHub's raw host serves this PDF as application/octet-stream. Require
    // a binary response, then verify the PDF signature and exact bytes below.
    let reference_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .trim()
        .to_owned();
    assert!(matches!(
        reference_type.as_str(),
        "application/pdf" | "application/octet-stream"
    ));
    assert!(
        response
            .content_length()
            .is_some_and(|n| n < 2 * 1024 * 1024)
    );
    let mut expected = Vec::new();
    while let Some(chunk) = response.chunk().await.unwrap() {
        assert!(expected.len() + chunk.len() < 2 * 1024 * 1024);
        expected.extend_from_slice(&chunk);
    }
    assert!(expected.starts_with(b"%PDF-"));
    assert_eq!(Sha256::digest(&bytes), Sha256::digest(&expected));
    let report = serde_json::json!({"page":PAGE,"profile":profile,"initial_status":initial.status,
        "candidates":discovery.files,"chosen_index":candidate,"status":saved.status,"saved_path":saved.saved_path,
        "bytes":bytes.len(),"sha256":format!("{:x}", Sha256::digest(&bytes)),"reference_content_type":reference_type,"independent_hash_match":true});
    std::fs::write(
        profile.join("result.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!("{}", serde_json::to_string(&report).unwrap());
    drop(scheduler);
}
