use download_engine::offline_website::{Config, Report, StopReason, save};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::{JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct Reply {
    status: u16,
    mime: String,
    body: Vec<u8>,
    location: Option<String>,
    stall: bool,
    unknown_length: bool,
}
impl Reply {
    fn new(mime: &str, bytes: impl AsRef<[u8]>) -> Self {
        Self {
            status: 200,
            mime: mime.into(),
            body: bytes.as_ref().to_vec(),
            location: None,
            stall: false,
            unknown_length: false,
        }
    }
    fn html(value: &str) -> Self {
        Self::new("text/html; charset=utf-8", value)
    }
    fn redirect(target: &str) -> Self {
        Self {
            status: 302,
            location: Some(target.into()),
            ..Self::html("")
        }
    }
}
struct Site {
    base: String,
    routes: Arc<Mutex<HashMap<String, Reply>>>,
    requests: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}
impl Drop for Site {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Site {
    async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let routes = Arc::new(Mutex::new(HashMap::<String, Reply>::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (data, log) = (routes.clone(), requests.clone());
        let task = tokio::spawn(async move {
            let mut workers = JoinSet::new();
            loop {
                tokio::select! {
                    connection = listener.accept() => {
                        let (mut stream, _) = connection.unwrap();
                        let (data, log) = (data.clone(), log.clone());
                        workers.spawn(async move {
                            let mut request = Vec::new();
                            while !request.ends_with(b"\r\n\r\n") && request.len() < 65536 { request.push(stream.read_u8().await?); }
                            let request = String::from_utf8_lossy(&request);
                            let path = request.split_whitespace().nth(1).unwrap_or("/");
                            log.lock().unwrap().push(path.to_string());
                            let reply = data.lock().unwrap().get(path).cloned().unwrap_or_else(|| Reply { status: 404, ..Reply::html("missing") });
                            if reply.stall { std::future::pending::<()>().await; }
                            let mut header = format!("HTTP/1.1 {} Fixture\r\nConnection: close\r\nContent-Type: {}\r\n", reply.status, reply.mime);
                            if !reply.unknown_length { header.push_str(&format!("Content-Length: {}\r\n", reply.body.len())); }
                            if let Some(location) = reply.location { header.push_str(&format!("Location: {location}\r\n")); }
                            header.push_str("\r\n");
                            stream.write_all(header.as_bytes()).await?;
                            stream.write_all(&reply.body).await?;
                            Ok::<_, std::io::Error>(())
                        });
                    }
                    _ = workers.join_next(), if !workers.is_empty() => {},
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
    fn put(&self, path: &str, reply: Reply) {
        self.routes.lock().unwrap().insert(path.into(), reply);
    }
    fn config(&self, path: &std::path::Path, depth: u8, pages: usize) -> Config {
        Config::new(&format!("{}/", self.base), path.to_path_buf(), depth, pages).unwrap()
    }
    fn seen(&self, url: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.as_str() == url)
            .count()
    }
}
fn file(report: &Report, ending: &str) -> std::path::PathBuf {
    report.folder.join(
        &report
            .files
            .iter()
            .find(|file| file.url.ends_with(ending))
            .unwrap()
            .path,
    )
}
fn text(path: &std::path::Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

#[tokio::test]
async fn linked_assets_are_saved_in_either_reference_order_without_false_omissions() {
    for links_first in [true, false] {
        let site = Site::new().await;
        let root = tempfile::tempdir().unwrap();
        let links = "<a href=/picture.png>Picture</a><a href=/sheet>Stylesheet</a>";
        let assets = "<img src=/picture.png><link rel=stylesheet href=/sheet>";
        site.put(
            "/",
            Reply::html(&if links_first {
                format!("{links}{assets}")
            } else {
                format!("{assets}{links}")
            }),
        );
        site.put("/picture.png", Reply::new("image/png", b"picture bytes"));
        site.put("/sheet", Reply::new("text/css", "body{color:blue}"));
        let report = save(
            site.config(root.path(), 1, 10),
            CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!(
            (report.pages, report.assets, report.issue_count),
            (1, 2, 0),
            "links_first={links_first}: {:?}",
            report.issues
        );
        let html = text(&report.entry());
        for ending in ["/picture.png", "/sheet"] {
            let local = file(&report, ending);
            assert!(html.contains(&format!(
                "href=\"{}\"",
                local.file_name().unwrap().to_string_lossy()
            )));
        }
        assert_eq!(site.seen("/picture.png"), 1);
        assert!(!html.contains("data:,") && !html.contains("report.html"));
    }
}

#[tokio::test]
async fn complete_static_copy_rewrites_navigation_css_images_queries_fragments_and_redirects() {
    use base64::Engine;
    let site = Site::new().await;
    let root = tempfile::tempdir().unwrap();
    site.put("/", Reply::redirect("/guide/index.html"));
    site.put("/guide/index.html", Reply::html(r##"<!doctype html><html><head><title>Offline fixture</title><base href="/guide/"><link rel=stylesheet href="/assets/main.css" integrity=old></head><body><h1>Offline fixture</h1><a id=next href="next.html?q=1#section">Next page</a><a href="next.html?q=2">Other query</a><a id=full-image href="/assets/pixel.png"><img id=hero width=32 height=32 src="/assets/pixel.png" srcset="/assets/pixel.png 1x, /assets/pixel.png?large=1 2x"></a><img id=vector src="/assets/vector.svg"><svg><a id=svg-next href="next.html?q=1#section"><set attributeName="href" to="https://offline-fixture.invalid/changed"/><text>SVG link</text></a></svg><div class=tile>Background</div><script src="/never.js">fetch('/never')</script></body></html>"##));
    for query in [1, 2] {
        site.put(&format!("/guide/next.html?q={query}"), Reply::html(&format!("<h1 id=section>Page {query}</h1><a id=back href='/guide/index.html'>Back</a><link rel=stylesheet href='/assets/main.css'>")));
    }
    site.put("/assets/main.css", Reply::new("text/css", "@import 'theme.css'; @font-face {font-family:Fixture;src:url(font.woff2)} .tile{background-image:url(pixel.png)} body{color:rgb(12,34,56)}"));
    site.put(
        "/assets/theme.css",
        Reply::new("text/css", "@import 'main.css'; h1{font-size:28px}"),
    );
    let png = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aZ1sAAAAASUVORK5CYII=").unwrap();
    site.put("/assets/pixel.png", Reply::new("image/png", &png));
    site.put("/assets/pixel.png?large=1", Reply::new("image/png", &png));
    site.put("/assets/vector.svg", Reply::new("image/svg+xml", br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><style>@im<![CDATA[port]]> &quot;https://offline-fixture.invalid/style.css&quot;;@namespace svg url(http://www.w3.org/2000/svg);svg|rect{fill:r&#103;b(0,128,0);stroke:u<![CDATA[rl]]>(https://offline-fixture.invalid/paint.svg)}</style><rect id="swatch" width="16" height="16"/></svg>"##));
    site.put(
        "/assets/font.woff2",
        Reply::new("font/woff2", b"fixture-font-bytes"),
    );
    let report = save(
        site.config(root.path(), 2, 20),
        CancellationToken::new(),
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!((report.pages, report.assets, report.issue_count), (3, 6, 1));
    assert!(report.issues[0].message.contains("SVG content was omitted"));
    assert_eq!(report.interactive_pages, 1);
    assert_eq!(site.seen("/assets/main.css"), 1);
    assert_eq!(site.seen("/never.js"), 0);
    assert_eq!(site.seen("/guide/index.html"), 1);
    let html = text(&report.entry());
    let next = file(&report, "next.html?q=1");
    assert!(html.contains(&format!(
        "{}#section",
        next.file_name().unwrap().to_string_lossy()
    )));
    assert_ne!(next, file(&report, "next.html?q=2"));
    assert!(text(&next).contains("href=\"index.html\""));
    assert!(!html.contains("<base") && !html.contains("<script") && !html.contains("integrity="));
    assert!(!html.contains("<set") && !html.contains("offline-fixture.invalid"));
    let svg = text(&file(&report, "/assets/vector.svg"));
    assert!(svg.contains("rgb(0,128,0)") && !svg.contains("offline-fixture.invalid"));
    assert_eq!(
        std::fs::read(file(&report, "/assets/pixel.png")).unwrap(),
        png
    );
    assert!(
        text(&file(&report, "/assets/main.css")).contains(
            file(&report, "/assets/theme.css")
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
        )
    );
    assert!(!text(&file(&report, "/assets/main.css")).contains("url(pixel.png)"));
    assert!(report.folder.join("report.json").exists());
    if let Ok(destination) = std::env::var("IDM_EX_OFFLINE_FIXTURE_OUTPUT") {
        let destination = std::path::PathBuf::from(destination);
        assert!(destination.is_absolute());
        std::fs::create_dir_all(&destination).unwrap();
        for entry in std::fs::read_dir(&report.folder).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
        }
    }
}

#[tokio::test]
async fn depth_page_limits_and_missing_assets_remain_browsable_and_reported() {
    let site = Site::new().await;
    let root = tempfile::tempdir().unwrap();
    site.put("/", Reply::html("<a href=/next>Next</a><a href=/other>Other</a><img src=/gone.png><a href=https://example.com/>External</a>"));
    site.put("/next", Reply::html("<h1>Next</h1><a href=/deep>Deep</a>"));
    let report = save(
        site.config(root.path(), 0, 10),
        CancellationToken::new(),
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!((report.pages, report.assets), (1, 0));
    assert_eq!(site.seen("/next"), 0);
    assert!(report.issues.iter().any(|i| i.message.contains("depth")));
    assert!(text(&report.entry()).contains("src=\"data:,\""));
    assert!(text(&report.entry()).contains("href=\"report.html\""));
    let limited = save(
        site.config(root.path(), 3, 2),
        CancellationToken::new(),
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(limited.pages, 2);
    assert!(
        limited
            .issues
            .iter()
            .any(|i| i.message.contains("Page limit"))
    );
    assert_ne!(limited.folder, report.folder);
}

#[tokio::test]
async fn cancelled_transfer_finalizes_partial_copy_without_waiting_for_stalled_server() {
    let site = Site::new().await;
    let root = tempfile::tempdir().unwrap();
    site.put("/", Reply::html("<a href=/slow>Slow</a>"));
    site.put(
        "/slow",
        Reply {
            stall: true,
            ..Reply::html("never")
        },
    );
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let task = tokio::spawn(save(site.config(root.path(), 1, 10), token, |_| {}));
    tokio::time::timeout(Duration::from_secs(5), async {
        while site.seen("/slow") == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    cancel.cancel();
    let report = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(report.stop, StopReason::Cancelled);
    assert_eq!(report.pages, 1);
    assert!(report.entry().exists() && report.folder.join("report.html").exists());
    assert!(text(&report.entry()).contains("href=\"report.html\""));
}

#[tokio::test]
async fn off_origin_redirects_and_external_local_assets_never_reach_the_other_server() {
    let site = Site::new().await;
    let other = Site::new().await;
    let root = tempfile::tempdir().unwrap();
    site.put(
        "/",
        Reply::html(&format!(
            "<img src='{}/private.png'><img src=/redirect.png><a href=/jump>Jump</a>",
            other.base
        )),
    );
    site.put(
        "/redirect.png",
        Reply::redirect(&format!("{}/private.png", other.base)),
    );
    site.put("/jump", Reply::redirect(&format!("{}/private", other.base)));
    let report = save(
        site.config(root.path(), 2, 20),
        CancellationToken::new(),
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(report.issue_count, 3);
    assert!(other.requests.lock().unwrap().is_empty());
    assert_eq!(report.pages, 1);
    assert!(
        report
            .issues
            .iter()
            .any(|i| i.message.contains("local or special"))
    );
}

#[tokio::test]
async fn size_limit_checks_streaming_bodies_and_keeps_existing_user_files() {
    let site = Site::new().await;
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("keep.html"), b"user content").unwrap();
    site.put("/", Reply::html("<img src=/huge.png>"));
    site.put(
        "/huge.png",
        Reply {
            unknown_length: true,
            ..Reply::new("image/png", vec![7; 2 * 1024 * 1024])
        },
    );
    let mut config = site.config(root.path(), 1, 10);
    config.megabytes = 1;
    let report = save(config, CancellationToken::new(), |_| {})
        .await
        .unwrap();
    assert_eq!(report.stop, StopReason::SizeLimit);
    assert_eq!(report.assets, 0);
    assert_eq!(text(&root.path().join("keep.html")), "user content");
    assert!(
        report
            .files
            .iter()
            .all(|f| !f.path.contains('/') && !f.path.contains('\\'))
    );
}

#[tokio::test]
async fn failed_or_cancelled_start_leaves_no_copy_and_invalid_config_never_connects() {
    let site = Site::new().await;
    let root = tempfile::tempdir().unwrap();
    site.put(
        "/",
        Reply::new("text/html; charset=windows-1252", "wrong encoding"),
    );
    let config = site.config(root.path(), 0, 10);
    let error = save(config.clone(), CancellationToken::new(), |_| {})
        .await
        .unwrap_err();
    assert!(error.to_string().contains("UTF-8"));
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(save(config.clone(), cancel, |_| {}).await.is_err());
    let mut invalid = config;
    invalid.megabytes = 0;
    assert!(
        save(invalid, CancellationToken::new(), |_| {})
            .await
            .is_err()
    );
    assert_eq!(site.seen("/"), 1);
}

#[tokio::test]
async fn redirected_self_link_does_not_consume_another_page_slot() {
    let site = Site::new().await;
    let root = tempfile::tempdir().unwrap();
    site.put("/", Reply::redirect("/home/"));
    site.put(
        "/home/",
        Reply::html("<a href='/home/#section'>Section</a><h1 id=section>Home</h1>"),
    );
    let report = save(
        site.config(root.path(), 1, 1),
        CancellationToken::new(),
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!((report.pages, report.issue_count), (1, 0));
    assert!(text(&report.entry()).contains("href=\"index.html#section\""));
}

#[tokio::test]
async fn hostile_names_and_active_content_cannot_escape_the_copy_or_fetch_scripts() {
    let site = Site::new().await;
    let root = tempfile::tempdir().unwrap();
    site.put("/", Reply::html("<a href='/%2e%2e/secret?name=../../outside'>Page</a><img src='/%2f..%2foutside.png'><script src=/payload.js></script><iframe src=/frame></iframe><meta http-equiv=refresh content='0; url=/bad'><form action=/submit><input name=password></form><a href='javascript:alert(1)'>Action</a>"));
    site.put("/secret?name=../../outside", Reply::html("Saved safely"));
    site.put(
        "/%2f..%2foutside.png",
        Reply::new("image/png", "image bytes"),
    );
    let report = save(
        site.config(root.path(), 1, 10),
        CancellationToken::new(),
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(report.pages, 2);
    for path in ["/payload.js", "/frame", "/submit", "/bad"] {
        assert_eq!(site.seen(path), 0);
    }
    let content = text(&report.entry());
    assert!(
        !content.contains("javascript:")
            && !content.contains("<script")
            && !content.contains("<iframe")
            && !content.contains("<form")
    );
    assert!(
        report
            .files
            .iter()
            .all(|f| report.folder.join(&f.path).parent().unwrap() == report.folder)
    );
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}

#[tokio::test]
async fn public_assets_can_be_disabled_without_changing_same_origin_assets() {
    let site = Site::new().await;
    let root = tempfile::tempdir().unwrap();
    site.put(
        "/",
        Reply::html("<img src='https://example.com/image.png'><img src=/local.png>"),
    );
    site.put("/local.png", Reply::new("image/png", "local image"));
    let mut config = site.config(root.path(), 1, 5);
    config.external_assets = false;
    let report = save(config, CancellationToken::new(), |_| {})
        .await
        .unwrap();
    assert_eq!(report.assets, 1);
    assert_eq!(report.issue_count, 1);
    assert!(report.issues[0].message.contains("disabled"));
}

#[tokio::test]
async fn proxy_snapshot_routes_pages_and_assets_and_never_falls_back_to_direct() {
    use download_engine::{
        paths::AppPaths,
        proxy::{LoginChange, Mode, Settings},
        store::Store,
    };
    let target = Site::new().await;
    let proxy = Site::new().await;
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: root.path().join("data"),
        downloads: root.path().join("downloads"),
    })
    .unwrap();
    let settings = Settings {
        mode: Mode::Custom,
        endpoint: proxy.base.clone(),
        ..Default::default()
    };
    store
        .save_proxy_settings(&Settings::default(), settings.clone(), LoginChange::Remove)
        .unwrap();
    proxy.put(
        &format!("{}/", target.base),
        Reply::html("<h1>Through proxy</h1><link rel=stylesheet href=/style.css>"),
    );
    proxy.put(
        &format!("{}/style.css", target.base),
        Reply::new("text/css", "body{color:blue}"),
    );
    let config = target.config(root.path(), 0, 5).with_proxy(&store).unwrap();
    store
        .save_proxy_settings(
            &store.proxy_settings().unwrap(),
            Settings::default(),
            LoginChange::Remove,
        )
        .unwrap();
    let report = save(config, CancellationToken::new(), |_| {})
        .await
        .unwrap();
    assert_eq!((report.pages, report.assets, report.issue_count), (1, 1, 0));
    assert!(target.requests.lock().unwrap().is_empty());
    assert_eq!(proxy.requests.lock().unwrap().len(), 2);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead = Settings {
        mode: Mode::Custom,
        endpoint: format!("http://{}", listener.local_addr().unwrap()),
        ..Default::default()
    };
    drop(listener);
    store
        .save_proxy_settings(&Settings::default(), dead, LoginChange::Remove)
        .unwrap();
    assert!(
        save(
            target.config(root.path(), 0, 5).with_proxy(&store).unwrap(),
            CancellationToken::new(),
            |_| {}
        )
        .await
        .is_err()
    );
    assert!(target.requests.lock().unwrap().is_empty());
    assert!(
        store
            .queue_jobs(download_engine::queue::MAIN_QUEUE)
            .unwrap()
            .is_empty()
    );
}
