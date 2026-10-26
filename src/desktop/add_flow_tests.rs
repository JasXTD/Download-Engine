//! Add forms exercised through rendered controls, the scheduler and real sockets.
use super::interaction_tests::{click, fixture, frame, text_rect};
use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use download_engine::proxy::{LoginChange, Mode, Settings};
use download_engine::queue::Padding;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

struct Server {
    base: String,
    bytes: Arc<Vec<u8>>,
    requests: Arc<Mutex<Vec<(String, bool)>>>,
    task: tokio::task::JoinHandle<()>,
    _runtime: tokio::runtime::Runtime,
}

impl Server {
    fn new() -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let listener = runtime.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let bytes = Arc::new(
            (0..1024 * 1024 + 47)
                .map(|i| (i % 251) as u8)
                .collect::<Vec<_>>(),
        );
        let requests = Arc::new(Mutex::new(Vec::new()));
        let body = bytes.clone();
        let observed = requests.clone();
        let task = runtime.spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    connection = listener.accept() => {
                        let (mut socket, _) = connection.unwrap();
                        let body = body.clone();
                        let observed = observed.clone();
                        connections.spawn(async move {
                            let mut header = Vec::new();
                            while !header.ends_with(b"\r\n\r\n") && header.len() < 32768 {
                                header.push(socket.read_u8().await?);
                            }
                            let header = String::from_utf8(header).unwrap();
                            let target = header.lines().next().unwrap().split_whitespace().nth(1).unwrap();
                            let lower = header.to_ascii_lowercase();
                            let expected = format!("Basic {}", STANDARD.encode("test-user:correct-password"));
                            let authenticated = header.lines().filter_map(|line| line.split_once(':')).any(|(name, value)| {
                                name.eq_ignore_ascii_case("authorization") && value.trim() == expected
                            });
                            observed.lock().unwrap().push((target.to_owned(), authenticated));
                            if target.ends_with("/private.bin") && !authenticated {
                                return socket.write_all(b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=fixture\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                            }
                            let range = lower.lines().find_map(|line| line.strip_prefix("range: bytes="));
                            let (start, end) = range.map(|range| {
                                let (start, end) = range.split_once('-').unwrap();
                                (start.parse::<usize>().unwrap(), end.parse::<usize>().unwrap())
                            }).unwrap_or((0, body.len() - 1));
                            let response = if range.is_some() {
                                format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{}\r\n", body.len())
                            } else {
                                "HTTP/1.1 200 OK\r\n".into()
                            };
                            socket.write_all(format!("{response}Content-Length: {}\r\nETag: \"add-flow\"\r\nConnection: close\r\n\r\n", end - start + 1).as_bytes()).await?;
                            socket.write_all(&body[start..=end]).await
                        });
                    },
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });
        Self {
            base,
            bytes,
            requests,
            task,
            _runtime: runtime,
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn with_app(test: impl FnOnce(&mut Desktop, &egui::Context, &Path)) {
    let (temp, mut app, ctx) = fixture();
    let store = app.store.clone();
    // Restore only this disposable profile and its generated vault IDs, even if
    // a UI assertion fails. Dropping Desktop first joins its scheduler workers.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        test(&mut app, &ctx, temp.path());
    }));
    drop(app);
    let jobs = store.list().unwrap();
    let mut credentials: Vec<_> = jobs
        .iter()
        .filter_map(|job| job.authentication.as_ref().map(|b| b.id))
        .collect();
    let proxy = store.proxy_settings().unwrap();
    credentials.extend(proxy.credential);
    for job in jobs {
        store.remove(&job.id).unwrap();
    }
    store
        .save_proxy_settings(&proxy, Settings::default(), LoginChange::Remove)
        .unwrap();
    for id in credentials {
        assert!(
            !auth::credentials_present(id).unwrap(),
            "Test login was not removed"
        );
    }
    let pending: i64 = rusqlite::Connection::open_with_flags(
        store.paths.data.join("queue.sqlite3"), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ).unwrap().query_row(
        "SELECT (SELECT COUNT(*) FROM credential_cleanup) + (SELECT COUNT(*) FROM proxy_credential_cleanup)",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(pending, 0, "Test credential cleanup is still pending");
    drop(store);
    temp.close().expect("Disposable UI profile was not removed");
    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}

fn until(app: &mut Desktop, ready: impl Fn(&Desktop) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        app.refresh();
        if ready(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "UI flow timed out; jobs: {:?}; form error: {:?}",
            app.store.list().unwrap(),
            app.form_error
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn enter(app: &mut Desktop, ctx: &egui::Context, id: &str, value: &str) {
    ctx.memory_mut(|memory| memory.request_focus(egui::Id::new(id)));
    frame(
        app,
        ctx,
        egui::vec2(1080.0, 800.0),
        vec![egui::Event::Text(value.into())],
    );
}

fn complete(app: &mut Desktop, count: usize, bytes: &[u8]) {
    until(app, |app| {
        let jobs = app.store.list().unwrap();
        assert!(
            jobs.iter().all(|job| job.status != Status::Failed),
            "{jobs:?}"
        );
        // Complete is persisted before crash-recovery staging links are removed.
        // Wait for the observable cleanup before asserting the destination contents.
        jobs.len() == count
            && jobs.iter().all(|job| {
                job.status == Status::Complete
                    && !job
                        .directory
                        .join(format!(".idm-ex-{}.assembling", job.id))
                        .try_exists()
                        .unwrap()
            })
    });
    for job in app.store.list().unwrap() {
        assert_eq!(std::fs::read(job.saved_path.unwrap()).unwrap(), bytes);
        assert_eq!(job.downloaded, bytes.len() as u64);
    }
}

#[test]
fn add_url_cancel_validation_and_custom_destination_complete_exact_file() {
    let server = Server::new();
    with_app(|app, ctx, root| {
        click(app, ctx, "Add URL");
        enter(
            app,
            ctx,
            "add-file-url",
            &format!("{}/public.bin", server.base),
        );
        click(app, ctx, "×");
        assert!(app.store.list().unwrap().is_empty());
        assert!(server.requests.lock().unwrap().is_empty());

        app.url.clear();
        click(app, ctx, "Add URL");
        enter(app, ctx, "add-file-url", "not-a-url");
        click(app, ctx, "Start download");
        assert!(app.form_error.is_some());
        assert_eq!(app.dialog, Some(Dialog::Add));
        assert!(app.store.list().unwrap().is_empty());

        app.url = format!("{}/public.bin", server.base);
        app.name = "renamed.bin".into();
        let destination = root.join("chosen-folder");
        app.folder = destination.to_string_lossy().into_owned();
        click(app, ctx, "Start download");
        complete(app, 1, &server.bytes);
        let job = app.store.list().unwrap().remove(0);
        assert_eq!(job.saved_path.unwrap(), destination.join("renamed.bin"));
        assert_eq!(std::fs::read_dir(destination).unwrap().count(), 1);
    });
}

#[test]
fn add_batch_rejects_invalid_lines_then_deduplicates_and_downloads_through_proxy() {
    let server = Server::new();
    with_app(|app, ctx, root| {
        app.store
            .save_proxy_settings(
                &Settings::default(),
                Settings {
                    mode: Mode::Custom,
                    endpoint: server.base.clone(),
                    ..Default::default()
                },
                LoginChange::Remove,
            )
            .unwrap();
        app.open_queue_dialog(Dialog::Batch);
        app.batch = "http://only-proxy.invalid/a.zip\nsftp://invalid.test/file".into();
        click(app, ctx, "Add files");
        assert_eq!(app.dialog, Some(Dialog::Batch));
        assert!(app.form_error.as_deref().unwrap().contains("Line 2"));
        assert!(app.store.list().unwrap().is_empty());
        assert!(server.requests.lock().unwrap().is_empty());

        app.batch = "  http://only-proxy.invalid/a.zip \n\nhttp://only-proxy.invalid/a.zip\r\nhttp://only-proxy.invalid/b.pdf\n".into();
        let destination = root.join("batch-folder");
        app.folder = destination.to_string_lossy().into_owned();
        click(app, ctx, "Add files");
        complete(app, 2, &server.bytes);
        assert!(app.dialog.is_none());
        assert!(app.batch.is_empty());
        assert_eq!(std::fs::read_dir(&destination).unwrap().count(), 2);
        for job in app.store.list().unwrap() {
            assert_eq!(job.directory, destination);
        }
        let requests = server.requests.lock().unwrap();
        assert!(requests.len() >= 4);
        assert!(
            requests
                .iter()
                .all(|(target, auth)| target.starts_with("http://only-proxy.invalid/") && !auth)
        );
    });
}

#[test]
fn add_batch_generates_numbered_links_into_the_selected_queue() {
    with_app(|app, ctx, root| {
        let photos = app.store.create_queue("Photos").unwrap();
        app.queue_id = photos.id.clone();
        app.force_refresh();
        app.batch = "https://example.test/list.zip".into();
        app.open_queue_dialog(Dialog::Batch);
        click(app, ctx, "From a pattern");
        assert!(app.batch_pattern.active);
        enter(
            app,
            ctx,
            "batch-pattern",
            "https://example.test/photos/img*.jpg",
        );
        app.batch_pattern.to.clear();
        enter(app, ctx, "batch-pattern-to", "1001");
        let size = egui::vec2(1080.0, 800.0);
        frame(app, ctx, size, vec![]);
        let output = frame(app, ctx, size, vec![]);
        text_rect(
            &output,
            "This pattern makes 1001 links. Add at most 1,000 per batch.",
        );
        click(app, ctx, "Add files");
        assert_eq!(app.dialog, Some(Dialog::Batch));
        assert!(app.form_error.as_deref().unwrap().contains("1001 links"));
        assert!(app.store.list().unwrap().is_empty());

        app.batch_pattern.to.clear();
        enter(app, ctx, "batch-pattern-to", "12");
        click(app, ctx, "Auto");
        click(app, ctx, "3");
        assert_eq!(app.batch_pattern.padding, Padding::Width(3));
        frame(app, ctx, size, vec![]);
        let output = frame(app, ctx, size, vec![]);
        for text in [
            "Preview · 12 links",
            "https://example.test/photos/img001.jpg",
            "https://example.test/photos/img012.jpg",
        ] {
            text_rect(&output, text);
        }
        let destination = root.join("photos");
        app.folder = destination.to_string_lossy().into_owned();
        click(app, ctx, "Add files");
        assert!(app.dialog.is_none(), "{:?}", app.form_error);
        let jobs = app.store.queue_jobs(&photos.id).unwrap();
        assert_eq!(
            jobs.iter().map(|job| job.url.clone()).collect::<Vec<_>>(),
            (1..=12)
                .map(|i| format!("https://example.test/photos/img{i:03}.jpg"))
                .collect::<Vec<_>>()
        );
        assert_eq!(app.store.list().unwrap().len(), 12);
        assert!(jobs.iter().all(|job| job.directory == destination));
        assert_eq!(jobs[0].name, "img001.jpg");
        assert_eq!(
            app.notice.as_ref().unwrap().message,
            "Added 12 files to Photos."
        );
        // The pasted list survives; the pattern is cleared but the mode is kept.
        assert_eq!(app.batch, "https://example.test/list.zip");
        assert!(app.batch_pattern.active && app.batch_pattern.pattern.is_empty());

        // Dropped links always open as a list for review.
        app.review_dropped_urls("https://example.test/dropped.zip")
            .unwrap();
        assert_eq!(app.dialog, Some(Dialog::Batch));
        assert!(!app.batch_pattern.active);
        let output = frame(app, ctx, size, vec![]);
        text_rect(&output, "File links");
    });
}

#[test]
fn inline_password_download_and_wrong_password_correction_keep_one_job() {
    for wrong_first in [false, true] {
        let server = Server::new();
        with_app(|app, ctx, _| {
            app.url = format!("{}/private.bin", server.base);
            click(app, ctx, "Add URL");
            click(app, ctx, "Advanced");
            click(app, ctx, "Use login");
            enter(app, ctx, "add-login-username", "test-user");
            enter(
                app,
                ctx,
                "add-login-secret",
                if wrong_first {
                    "wrong-password"
                } else {
                    "correct-password"
                },
            );
            assert_eq!(&*app.add_login.username, "test-user");
            assert_eq!(
                &*app.add_login.secret,
                if wrong_first {
                    "wrong-password"
                } else {
                    "correct-password"
                }
            );
            click(app, ctx, "Start download");
            if wrong_first {
                until(app, |app| {
                    app.store
                        .list()
                        .unwrap()
                        .iter()
                        .any(|job| job.status == Status::Failed)
                });
                let failed = app.store.list().unwrap().remove(0);
                let old_credential = failed.authentication.as_ref().unwrap().id;
                assert_eq!(failed.downloaded, 0);
                assert!(failed.error.as_deref().unwrap().contains("401"));
                assert!(
                    !serde_json::to_string(&failed)
                        .unwrap()
                        .contains("wrong-password")
                );
                app.open_authentication(&failed);
                enter(app, ctx, "authentication-username", "test-user");
                enter(app, ctx, "authentication-secret", "correct-password");
                click(app, ctx, "Save and continue");
                assert!(app.authentication.as_ref().unwrap().request.is_some());
                // The failed row remains visible until the asynchronous save
                // finishes. Wait for its correlated result before completion.
                until(app, |app| app.authentication.is_none());
                complete(app, 1, &server.bytes);
                assert_eq!(app.store.list().unwrap()[0].id, failed.id);
                assert!(!auth::credentials_present(old_credential).unwrap());
            } else {
                complete(app, 1, &server.bytes);
            }
            until(app, |app| {
                app.store.list().unwrap()[0].authentication.is_none()
            });
            assert!(app.add_login.secret.is_empty() && app.add_login.username.is_empty());
            assert!(
                server
                    .requests
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|(_, authenticated)| *authenticated)
            );
        });
    }
}
