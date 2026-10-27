use download_engine::{
    browser_batch::{Destination, Link},
    categories::Category,
    engine,
    paths::AppPaths,
    queue::MAIN_QUEUE,
    store::Store,
    website_grabber::{Config, FileTypes, Report, StopReason, scan},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Notify,
    task::{JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    bytes: Vec<u8>,
    stall_headers: bool,
    stall_body: bool,
    unknown_length: bool,
}
impl Response {
    fn html(text: impl AsRef<str>) -> Self {
        Self {
            status: 200,
            headers: vec![("Content-Type".into(), "text/html; charset=utf-8".into())],
            bytes: text.as_ref().as_bytes().to_vec(),
            stall_headers: false,
            stall_body: false,
            unknown_length: false,
        }
    }
    fn redirect(url: &str) -> Self {
        let mut response = Self::html("");
        response.status = 302;
        response.headers.push(("Location".into(), url.into()));
        response
    }
    fn file(bytes: &[u8]) -> Self {
        let mut response = Self::html("");
        response.bytes = bytes.into();
        response.headers = vec![("Content-Type".into(), "application/pdf".into())];
        response
    }
}
struct Server {
    base: String,
    routes: Arc<Mutex<HashMap<String, Response>>>,
    requests: Arc<Mutex<Vec<String>>>,
    arrived: Arc<Notify>,
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
        let routes = Arc::new(Mutex::new(HashMap::<String, Response>::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let arrived = Arc::new(Notify::new());
        let (data, log, signal) = (routes.clone(), requests.clone(), arrived.clone());
        let task = tokio::spawn(async move {
            let mut handlers = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (mut stream, _) = accepted.unwrap();
                        let (data, log, signal) = (data.clone(), log.clone(), signal.clone());
                        handlers.spawn(async move {
                            let mut bytes = Vec::new();
                            while !bytes.ends_with(b"\r\n\r\n") && bytes.len() < 65536 { bytes.push(stream.read_u8().await?); }
                            let request = String::from_utf8_lossy(&bytes).to_string();
                            let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                            let response = data.lock().unwrap().get(&path).cloned().unwrap_or_else(|| { let mut r = Response::html("not found"); r.status = 404; r });
                            log.lock().unwrap().push(request);
                            signal.notify_one();
                            if response.stall_headers { std::future::pending::<()>().await; }
                            let mut headers = format!("HTTP/1.1 {} Fixture\r\nConnection: close\r\n", response.status);
                            if !response.unknown_length { headers.push_str(&format!("Content-Length: {}\r\n", response.bytes.len())); }
                            for (name, value) in response.headers { headers.push_str(&format!("{name}: {value}\r\n")); }
                            headers.push_str("\r\n");
                            stream.write_all(headers.as_bytes()).await?;
                            if response.stall_body { std::future::pending::<()>().await; }
                            stream.write_all(&response.bytes).await?;
                            Ok::<_, std::io::Error>(())
                        });
                    }
                    _ = handlers.join_next(), if !handlers.is_empty() => {},
                }
            }
        });
        Self {
            base,
            routes,
            requests,
            arrived,
            task,
        }
    }
    fn put(&self, path: &str, response: Response) {
        self.routes.lock().unwrap().insert(path.into(), response);
    }
    fn paths(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.split_whitespace().nth(1).unwrap().into())
            .collect()
    }
    async fn scan(&self, depth: u8, pages: usize, types: &str) -> Report {
        scan(
            Config::new(&format!("{}/", self.base), types, depth, pages).unwrap(),
            CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap()
    }
}
fn names(report: &Report) -> Vec<&str> {
    report.files.iter().map(|f| f.name.as_str()).collect()
}

#[tokio::test]
async fn github_file_previews_use_raw_urls_and_deduplicate_the_download() {
    let s = Server::new().await;
    s.put("/", Response::html(r#"
        <a href="https://github.com/owner/repo/blob/main/guide.pdf">View PDF</a>
        <a href="https://github.com/owner/repo/raw/main/guide.pdf">Raw PDF</a>
        <a href="https://github.com/owner/repo/blob/feature/topic/a%20b.pdf?download=1#page2">Branch file</a>
        <a href="https://github.com.example.test/owner/repo/blob/main/guide.pdf">Other host</a>
        <a href="https://example.test/blob/main/guide.pdf">Other website</a>
        <a href="https://github.com/owner/repo/releases/download/v1/file.zip">Release</a>
        <a href="https://github.com/owner/repo/commits/main/guide.pdf">History</a>
        <a href="https://github.com/owner/repo/commit/abc/guide.pdf">Commit</a>
        <a href="https://github.com/owner/repo/blame/main/guide.pdf">Blame</a>
        <a href="https://github.com/owner/repo/edit/main/guide.pdf">Edit</a>
        <a href="https://github.com/owner/repo/delete/main/guide.pdf">Delete</a>
        <a href="https://github.com/owner/repo/tree/main/guide.pdf">Tree</a>
    "#));
    let report = s.scan(0, 1, "pdf,zip").await;
    assert_eq!(report.files.len(), 5);
    assert_eq!(
        report.files[0].url,
        "https://github.com/owner/repo/raw/main/guide.pdf"
    );
    assert_eq!(
        report.files[1].url,
        "https://github.com/owner/repo/raw/feature/topic/a%20b.pdf?download=1"
    );
    assert_eq!(
        report.files[2].url,
        "https://github.com.example.test/owner/repo/blob/main/guide.pdf"
    );
    assert_eq!(
        report.files[3].url,
        "https://example.test/blob/main/guide.pdf"
    );
    assert_eq!(
        report.files[4].url,
        "https://github.com/owner/repo/releases/download/v1/file.zip"
    );
    assert!(
        report
            .files
            .iter()
            .all(|f| f.page == format!("{}/", s.base))
    );
    assert_eq!(s.paths(), ["/"]); // Discovery never fetches the files.
}

#[tokio::test]
async fn html_fragments_are_bounded_page_content_without_following_navigation() {
    let s = Server::new().await;
    let other = Server::new().await;
    s.put(
        "/",
        Response::html(format!(
            r#"
        <a href="/unrelated">Navigation</a><a href="one.zip">One</a>
        <include-fragment src="/assets.v1"></include-fragment>
        <include-fragment src="/assets.v1#again"></include-fragment>
        <include-fragment src="{}/external"></include-fragment>
    "#,
            other.base
        )),
    );
    s.put(
        "/assets.v1",
        Response::html(
            r#"
        <a href="two.zip">Two</a><a href="/unrelated">Navigation</a>
        <include-fragment src="/"></include-fragment>
        <script src="/never.js"></script>
    "#,
        ),
    );
    let bounded = s.scan(0, 1, "zip").await;
    assert_eq!(names(&bounded), ["one.zip"]);
    assert_eq!(bounded.stop, StopReason::PageLimit);
    s.requests.lock().unwrap().clear();
    let full = s.scan(0, 2, "zip").await;
    assert_eq!(names(&full), ["one.zip", "two.zip"]);
    assert_eq!(full.pages_read, 2);
    assert_eq!(full.stop, StopReason::Finished);
    assert_eq!(s.paths(), ["/", "/assets.v1"]);
    assert!(other.paths().is_empty());
}

#[tokio::test]
async fn custom_html_fragments_use_the_declared_type_and_skip_non_html_formats() {
    let s = Server::new().await;
    s.put(
        "/",
        Response::html(
            r#"
        <include-fragment src="/html.v1" accept="text/fragment+html"></include-fragment>
        <include-fragment src="/data" accept="application/json"></include-fragment>
    "#,
        ),
    );
    let mut fragment = Response::html("<a href='/manual.pdf'>Manual</a>");
    fragment.headers[0].1 = "text/fragment+html; charset=utf-8".into();
    s.put("/html.v1", fragment);
    let report = s.scan(0, 5, "pdf").await;
    assert_eq!(names(&report), ["manual.pdf"]);
    assert_eq!(report.pages_failed, 0);
    assert_eq!(s.paths(), ["/", "/html.v1"]);
    assert!(
        s.requests.lock().unwrap()[1]
            .to_ascii_lowercase()
            .contains("accept: text/fragment+html\r\n")
    );
}

#[tokio::test]
async fn scheduled_project_discovers_while_queue_paused_then_downloads_only_reviewed_file() {
    use download_engine::{grabber_projects::Settings, model::Status, scheduler::Scheduler};
    let server = Server::new().await;
    server.put(
        "/",
        Response::html("<a href='manual.pdf'>manual</a><a href='leave.pdf'>leave</a>"),
    );
    server.put("/manual.pdf", Response::file(b"scheduled fixture document"));
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    store.set_queue_running(MAIN_QUEUE, false).unwrap();
    let project = store
        .create_grabber_project(
            "Automatic manuals",
            &Settings {
                url: format!("{}/", server.base),
                extensions: "pdf".into(),
                depth: 0,
                page_limit: 1,
                queue: MAIN_QUEUE.into(),
                destination: Destination::Documents,
            },
        )
        .unwrap();
    let now = chrono::Utc::now().timestamp();
    // A persisted occurrence that became due while the desktop was closed.
    store
        .set_grabber_schedule(&project, now - 1, None, now - 2)
        .unwrap();
    let scheduler = Scheduler::start(store.clone()).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    let discoveries = loop {
        assert!(scheduler.errors.try_recv().is_err());
        let files = store.grabber_discoveries(&project.id).unwrap();
        if files.len() == 2 {
            break files;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "scheduled scan did not finish"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(server.paths(), ["/"]);
    assert!(store.list().unwrap().is_empty());
    let ids = store
        .review_grabber_files(
            &project.id,
            &[discoveries[0].id.clone()],
            Some((MAIN_QUEUE, Destination::Documents)),
        )
        .unwrap();
    assert!(
        store.queues().unwrap()[0].running,
        "Reviewing a file must wake an idle, unscheduled Main queue"
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    let completed = loop {
        let job = store.get(&ids[0]).unwrap().unwrap();
        assert_ne!(job.status, Status::Failed, "{:?}", job.error);
        if job.status == Status::Complete {
            break job;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "reviewed download did not finish"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(
        std::fs::read(completed.saved_path.unwrap()).unwrap(),
        b"scheduled fixture document"
    );
    assert!(!server.paths().iter().any(|path| path == "/leave.pdf"));
    drop(scheduler);
    let reopened = Store::open(store.paths.clone()).unwrap();
    assert_eq!(
        reopened.grabber_discoveries(&project.id).unwrap()[0]
            .file
            .name,
        "leave.pdf"
    );
    assert!(!reopened.grabber_schedules().unwrap()[0].enabled);
}

#[test]
fn validates_scope_depth_and_extension_filters_before_any_network_work() {
    for types in ["", "*", "*.*", "pdf/zip", "p?f", "tar..gz", "pdf:\\", "."] {
        assert!(FileTypes::parse(types).is_err(), "{types}");
    }
    assert!(FileTypes::parse("*.PDF, .zip; TAR.GZ png").is_ok());
    for url in [
        "file:///C:/secret",
        "ftp://example.com",
        "https://user:pass@example.com/",
    ] {
        assert!(Config::new(url, "zip", 1, 20).is_err());
    }
    assert!(Config::new("https://example.com", "zip", 6, 20).is_err());
    assert!(Config::new("https://example.com", "zip", 1, 0).is_err());
    assert!(Config::new("https://example.com", "zip", 1, 201).is_err());
}

#[tokio::test]
async fn depth_is_breadth_first_and_files_are_previewed_without_fetching_them() {
    let s = Server::new().await;
    s.put("/", Response::html(r#"<a href="guide.PDF?version=1#one">pdf</a><a href="guide.PDF?version=1#two">duplicate</a>
        <a href="child/">child</a><a href="child/#again">same child</a><a href="wrong.exe">not selected</a>
        <img src="cover%2Epng"><a href="download?id=9&amp;v=1" download="report.pdf">opaque URL</a>"#));
    s.put(
        "/child/",
        Response::html(
            "<a href='../'>cycle</a><a href='next/'>next</a><a href='child.zip'>file</a>",
        ),
    );
    s.put(
        "/child/next/",
        Response::html("<a href='deep.pdf'>deep</a>"),
    );
    let zero = s.scan(0, 50, "pdf zip png").await;
    assert_eq!(names(&zero), ["guide.PDF", "cover.png", "report.pdf"]);
    assert_eq!(zero.files[2].url, format!("{}/download?id=9&v=1", s.base));
    assert_eq!(zero.files[2].filename.as_deref(), Some("report.pdf"));
    assert_eq!(s.paths(), ["/"]);
    s.requests.lock().unwrap().clear();
    let one = s.scan(1, 50, "pdf zip png").await;
    assert_eq!(one.pages_read, 2);
    assert_eq!(one.files.len(), 4);
    assert_eq!(s.paths(), ["/", "/child/"]);
    s.requests.lock().unwrap().clear();
    let two = s.scan(2, 50, "pdf zip png").await;
    assert_eq!(two.files.len(), 5);
    assert_eq!(s.paths(), ["/", "/child/", "/child/next/"]);
    assert_eq!(two.stop, StopReason::Finished);
}

#[tokio::test]
async fn follows_html_base_handles_entities_and_never_executes_page_content() {
    let s = Server::new().await;
    s.put("/", Response::html(r#"<base href="/assets/"><A HREF="Guide%2Epdf?x=1&amp;y=2">guide</A>
        <source src="video.mp4"><a href="pack.tar.gz">archive</a><a href="/opaque" download="../../CON.pdf">file</a>
        <a href="javascript:alert(1)">no</a><a href="file:///C:/private.pdf">no</a><a href="https://u:p@example.test/private.pdf">no</a>
        <form action="/submit"><input type="submit"></form><script>document.write('<a href="script.pdf">')</script>
        <iframe src="/frame"></iframe><a href="/download" download>unknown type</a>"#));
    let r = s.scan(3, 50, "pdf, tar.gz, mp4").await;
    assert_eq!(r.files.len(), 4);
    assert_eq!(
        r.files[0].url,
        format!("{}/assets/Guide%2Epdf?x=1&y=2", s.base)
    );
    assert!(r.files[3].name.ends_with("CON.pdf"));
    assert!(!r.files[3].name.contains('/'));
    assert_eq!(s.paths(), ["/"]);
}

#[tokio::test]
async fn matching_html_files_remain_navigable_at_the_selected_depth() {
    let s = Server::new().await;
    s.put(
        "/",
        Response::html(
            "<a href='child.html' download>save page</a><a href='child.html'>visit page</a>",
        ),
    );
    s.put(
        "/child.html",
        Response::html("<a href='manual.pdf'>file</a><a href='deep.html'>deeper</a>"),
    );
    s.put(
        "/deep.html",
        Response::html("<a href='too-deep.pdf'>file</a>"),
    );
    let r = s.scan(1, 50, "html, pdf").await;
    assert_eq!(names(&r), ["child.html", "manual.pdf", "deep.html"]);
    assert_eq!(s.paths(), ["/", "/child.html"]);
}

#[tokio::test]
async fn external_files_are_preview_only_and_cross_origin_redirects_are_not_followed() {
    let s = Server::new().await;
    let external = Server::new().await;
    s.put("/", Response::html(format!("<a href='{0}/private/'>external page</a><a href='{0}/public.pdf'>external file</a><a href='/redirect'>redirect</a><a href='/local'>local</a>", external.base)));
    s.put(
        "/redirect",
        Response::redirect(&format!("{}/private/", external.base)),
    );
    let mut local = Response::html("<a href='/local.pdf'>local</a><a href='/after/'>next page</a>");
    local.headers.push((
        "Set-Cookie".into(),
        "session=must-not-be-sent; Path=/".into(),
    ));
    s.put("/local", local);
    s.put("/after/", Response::html("No further links"));
    let r = s.scan(2, 50, "pdf").await;
    assert_eq!(r.files.len(), 2);
    assert_eq!(r.pages_failed, 1);
    assert!(r.issues[0].message.contains("redirect"));
    assert!(external.paths().is_empty());
    assert!(s.paths().contains(&"/after/".into()));
    for request in s.requests.lock().unwrap().iter() {
        let request = request.to_ascii_lowercase();
        assert!(
            !request.contains("cookie:")
                && !request.contains("authorization:")
                && !request.contains("referer:")
        );
    }
}

#[tokio::test]
async fn page_and_file_limits_keep_partial_results_and_bound_requests() {
    let s = Server::new().await;
    s.put(
        "/",
        Response::html("<a href='root.pdf'>root</a><a href='a/'>a</a><a href='b/'>b</a>"),
    );
    s.put("/a/", Response::html("<a href='a.pdf'>a file</a>"));
    s.put("/b/", Response::html("<a href='b.pdf'>b file</a>"));
    let r = s.scan(3, 2, "pdf").await;
    assert_eq!(r.stop, StopReason::PageLimit);
    assert_eq!(s.paths(), ["/", "/a/"]);
    assert_eq!(r.files.len(), 2);
    s.put(
        "/",
        Response::html(
            (0..1100)
                .map(|i| format!("<a href='{i}.pdf'>file</a>"))
                .collect::<String>(),
        ),
    );
    let r = s.scan(0, 50, "pdf").await;
    assert_eq!(r.files.len(), 1000);
    assert_eq!(r.stop, StopReason::FileLimit);
}

#[tokio::test]
async fn redirect_cycles_errors_and_oversized_pages_do_not_erase_other_matches() {
    let s = Server::new().await;
    s.put("/", Response::html("<a href='root.zip'>ok</a><a href='loop/'>loop</a><a href='large/'>large</a><a href='missing/'>404</a><a href='binary/'>not HTML</a>"));
    s.put("/loop/", Response::redirect("/loop/"));
    let mut large = Response::html(" ".repeat(2 * 1024 * 1024 + 1));
    large.unknown_length = true; // Exercise streamed size enforcement, not just Content-Length.
    s.put("/large/", large);
    s.put("/binary/", Response::file(b"not html"));
    let r = s.scan(1, 50, "zip").await;
    assert_eq!(names(&r), ["root.zip"]);
    assert_eq!(r.pages_failed, 4);
    assert!(r.issues.iter().any(|i| i.message.contains("2 MiB")));
    assert!(s.paths().len() <= 10);
}

#[tokio::test]
async fn stopping_a_stalled_page_returns_partial_results_promptly() {
    for body in [false, true] {
        let s = Server::new().await;
        s.put(
            "/",
            Response::html("<a href='root.pdf'>file</a><a href='slow/'>slow</a>"),
        );
        let mut slow = Response::html("never sent");
        slow.stall_body = body;
        slow.stall_headers = !body;
        s.put("/slow/", slow);
        let cancel = CancellationToken::new();
        let config = Config::new(&s.base, "pdf", 1, 50).unwrap();
        let token = cancel.clone();
        let task = tokio::spawn(scan(config, token, |_| {}));
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if s.paths().contains(&"/slow/".into()) {
                    break;
                }
                s.arrived.notified().await;
            }
        })
        .await
        .unwrap();
        cancel.cancel();
        let r = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(r.stop, StopReason::Cancelled);
        assert_eq!(names(&r), ["root.pdf"]);
    }
}

#[tokio::test]
async fn cancelling_before_start_makes_no_requests() {
    let s = Server::new().await;
    let cancel = CancellationToken::new();
    cancel.cancel();
    let r = scan(Config::new(&s.base, "pdf", 0, 1).unwrap(), cancel, |_| {})
        .await
        .unwrap();
    assert_eq!(r.stop, StopReason::Cancelled);
    assert!(s.paths().is_empty());
}

#[tokio::test]
async fn stalled_pages_time_out_without_losing_results() {
    let s = Server::new().await;
    s.put(
        "/",
        Response::html("<a href='root.pdf'>file</a><a href='slow/'>slow</a>"),
    );
    let mut slow = Response::html("never sent");
    slow.stall_headers = true;
    s.put("/slow/", slow);
    let task = tokio::spawn(scan(
        Config::new(&s.base, "pdf", 1, 50).unwrap(),
        CancellationToken::new(),
        |_| {},
    ));
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if s.paths().contains(&"/slow/".into()) {
                break;
            }
            s.arrived.notified().await;
        }
    })
    .await
    .unwrap();
    // The request is already in flight; advance its real timeout deterministically.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(16)).await;
    let r = task.await.unwrap().unwrap();
    tokio::time::resume();
    assert_eq!(r.pages_failed, 1);
    assert!(
        r.issues[0].message.contains("timed out"),
        "{}",
        r.issues[0].message
    );
    assert_eq!(names(&r), ["root.pdf"]);
}

#[tokio::test]
async fn scan_byte_budget_and_preview_byte_budget_are_bounded() {
    let s = Server::new().await;
    let links = (0..15)
        .map(|i| format!("<a href='{i}/'>page</a>"))
        .collect::<String>();
    s.put("/", Response::html(links));
    for i in 0..15 {
        s.put(
            &format!("/{i}/"),
            Response::html(" ".repeat(2 * 1024 * 1024)),
        );
    }
    let r = s.scan(1, 50, "pdf").await;
    assert_eq!(r.stop, StopReason::SizeLimit);
    assert!(s.paths().len() <= 11);
    s.put(
        "/",
        Response::html(
            (0..100)
                .map(|i| format!("<a href='{i}.pdf?key={}'>file</a>", "x".repeat(9000)))
                .collect::<String>(),
        ),
    );
    let r = s.scan(0, 50, "pdf").await;
    assert_eq!(r.stop, StopReason::SizeLimit);
    assert!(r.files.len() > 50 && r.files.len() < 100);
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: dir.path().join("data"),
        downloads: dir.path().join("downloads"),
    })
    .unwrap();
    let links: Vec<_> = r
        .files
        .into_iter()
        .map(|f| Link {
            url: f.url,
            filename: f.filename,
        })
        .collect();
    // Even the maximum-size preview fits the existing batch contract.
    assert_eq!(
        store
            .enqueue_browser_batch(
                &format!("batch:{}", uuid::Uuid::new_v4()),
                &links,
                MAIN_QUEUE,
                Destination::Automatic
            )
            .unwrap()
            .len(),
        links.len()
    );
}

#[tokio::test]
async fn malformed_text_is_reported_instead_of_silently_changing_urls() {
    let s = Server::new().await;
    let mut response = Response::html("");
    response.bytes = b"<a href='\xff.pdf'>invalid UTF-8</a>".to_vec();
    s.put("/", response);
    let r = s.scan(0, 1, "pdf").await;
    assert!(r.files.is_empty());
    assert_eq!(r.pages_failed, 1);
    assert!(r.issues[0].message.contains("UTF-8"));
}

#[tokio::test]
async fn preview_selection_queues_atomically_and_downloads_exact_bytes_with_folder_rules() {
    let s = Server::new().await;
    s.put(
        "/",
        Response::html("<a href='manual.pdf'>manual</a><a href='skip.pdf'>skip</a>"),
    );
    s.put("/manual.pdf", Response::file(b"fixture document bytes"));
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    let queue = store.create_queue("Grabbed files").unwrap();
    let mut folders = store.download_folders().unwrap();
    folders
        .overrides
        .insert(Category::Documents, temp.path().join("custom-documents"));
    store.save_download_folders(&folders).unwrap();
    let report = s.scan(0, 50, "pdf").await;
    assert!(store.queue_jobs(&queue.id).unwrap().is_empty());
    assert_eq!(s.paths(), ["/"]);
    let selected = vec![Link {
        url: report.files[0].url.clone(),
        filename: report.files[0].filename.clone(),
    }];
    let receipt = format!("batch:{}", uuid::Uuid::new_v4());
    let ids = store
        .enqueue_browser_batch(&receipt, &selected, &queue.id, Destination::Automatic)
        .unwrap();
    assert_eq!(
        ids,
        store
            .enqueue_browser_batch(&receipt, &selected, &queue.id, Destination::Automatic)
            .unwrap()
    );
    assert_eq!(store.queue_jobs(&queue.id).unwrap().len(), 1);
    assert!(store.queue_jobs(MAIN_QUEUE).unwrap().is_empty());
    assert!(
        !store
            .queues()
            .unwrap()
            .iter()
            .find(|q| q.id == queue.id)
            .unwrap()
            .running
    );
    let mut job = store.get(&ids[0]).unwrap().unwrap();
    assert_eq!(job.directory, temp.path().join("custom-documents"));
    assert!(job.authentication.is_none());
    job.status = download_engine::model::Status::Downloading;
    store.save(&job).unwrap();
    engine::run(
        store.clone(),
        job,
        CancellationToken::new(),
        download_engine::bandwidth::Limiter::default(),
    )
    .await
    .unwrap();
    let complete = store.get(&ids[0]).unwrap().unwrap();
    assert_eq!(
        complete.status,
        download_engine::model::Status::Complete,
        "{:?}",
        complete.error
    );
    assert_eq!(
        tokio::fs::read(complete.saved_path.unwrap()).await.unwrap(),
        b"fixture document bytes"
    );
    assert!(!s.paths().iter().any(|p| p == "/skip.pdf"));
}
