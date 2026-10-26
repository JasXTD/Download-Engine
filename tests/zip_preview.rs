use download_engine::{
    model::{Job, Status},
    paths::AppPaths,
    proxy::{LoginChange, Mode, Settings},
    store::Store,
    zip_preview,
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::{JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

fn put16(b: &mut [u8], p: usize, value: u16) {
    b[p..p + 2].copy_from_slice(&value.to_le_bytes());
}
fn put32(b: &mut [u8], p: usize, value: u32) {
    b[p..p + 4].copy_from_slice(&value.to_le_bytes());
}
fn put64(b: &mut [u8], p: usize, value: u64) {
    b[p..p + 8].copy_from_slice(&value.to_le_bytes());
}
struct Archive {
    bytes: Vec<u8>,
    central: usize,
    end: usize,
}
fn archive(count: usize, data_size: usize, zip64: bool) -> Archive {
    let mut bytes = Vec::new();
    let mut directory = Vec::new();
    for i in 0..count {
        let name = format!("directory/entry-{i:05}.txt");
        let content_size = if i == 0 { data_size } else { 1 };
        let offset = bytes.len();
        let mut local = vec![0; 30];
        local[0..4].copy_from_slice(b"PK\x03\x04");
        put16(&mut local, 4, 20);
        put16(&mut local, 6, 0x800);
        put32(&mut local, 18, content_size as u32);
        put32(&mut local, 22, content_size as u32);
        put16(&mut local, 26, name.len() as u16);
        bytes.extend(local);
        bytes.extend(name.as_bytes());
        bytes.extend((0..content_size).map(|n| (n % 251) as u8));
        let mut entry = vec![0; 46];
        entry[0..4].copy_from_slice(b"PK\x01\x02");
        put16(&mut entry, 4, 20);
        put16(&mut entry, 6, if zip64 { 45 } else { 20 });
        put16(&mut entry, 8, 0x800);
        put32(
            &mut entry,
            20,
            if zip64 { u32::MAX } else { content_size as u32 },
        );
        put32(
            &mut entry,
            24,
            if zip64 { u32::MAX } else { content_size as u32 },
        );
        put16(&mut entry, 28, name.len() as u16);
        put32(&mut entry, 42, if zip64 { u32::MAX } else { offset as u32 });
        if zip64 {
            put16(&mut entry, 30, 28);
        }
        directory.extend(entry);
        directory.extend(name.as_bytes());
        if zip64 {
            let mut extra = vec![0; 28];
            put16(&mut extra, 0, 1);
            put16(&mut extra, 2, 24);
            put64(&mut extra, 4, content_size as u64);
            put64(&mut extra, 12, content_size as u64);
            put64(&mut extra, 20, offset as u64);
            directory.extend(extra);
        }
    }
    let central = bytes.len();
    let length = directory.len();
    bytes.extend(directory);
    if zip64 {
        let offset = bytes.len();
        let mut record = vec![0; 56];
        record[0..4].copy_from_slice(b"PK\x06\x06");
        put64(&mut record, 4, 44);
        put16(&mut record, 12, 45);
        put16(&mut record, 14, 45);
        put64(&mut record, 24, count as u64);
        put64(&mut record, 32, count as u64);
        put64(&mut record, 40, length as u64);
        put64(&mut record, 48, central as u64);
        bytes.extend(record);
        let mut locator = vec![0; 20];
        locator[0..4].copy_from_slice(b"PK\x06\x07");
        put64(&mut locator, 8, offset as u64);
        put32(&mut locator, 16, 1);
        bytes.extend(locator);
    }
    let end = bytes.len();
    let mut record = vec![0; 22];
    record[0..4].copy_from_slice(b"PK\x05\x06");
    put16(&mut record, 8, if zip64 { u16::MAX } else { count as u16 });
    put16(&mut record, 10, if zip64 { u16::MAX } else { count as u16 });
    put32(
        &mut record,
        12,
        if zip64 { u32::MAX } else { length as u32 },
    );
    put32(
        &mut record,
        16,
        if zip64 { u32::MAX } else { central as u32 },
    );
    bytes.extend(record);
    Archive {
        bytes,
        central,
        end,
    }
}

struct Server {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn new(bytes: Vec<u8>, mode: u8) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/archive.zip", listener.local_addr().unwrap());
        let mode = Arc::new(AtomicU8::new(mode));
        let server_mode = mode.clone();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        let bytes = Arc::new(bytes);
        let task = tokio::spawn(async move {
            let mut tasks = JoinSet::new();
            loop {
                tokio::select! {
                    result = listener.accept() => {
                        let (mut socket, _) = result.unwrap(); let bytes = bytes.clone(); let seen = seen.clone(); let mode = server_mode.load(Ordering::SeqCst);
                        tasks.spawn(async move {
                            let mut raw = Vec::new();
                            while !raw.ends_with(b"\r\n\r\n") {
                                match socket.read_u8().await { Ok(b) => raw.push(b), Err(_) => return }
                                assert!(raw.len() < 16384);
                            }
                            let request = String::from_utf8(raw).unwrap(); let lower = request.to_lowercase();
                            let number = { let mut seen = seen.lock().unwrap(); seen.push(request); seen.len() };
                            if mode == 10 { std::future::pending::<()>().await; }
                            if mode == 7 { let _ = socket.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n").await; return; }
                            if mode == 12 && !lower.contains("authorization: basic dXNlcjpwYXNz".to_lowercase().as_str()) {
                                let _ = socket.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n").await; return;
                            }
                            let range = lower.lines().find_map(|l| l.strip_prefix("range: bytes=")).unwrap();
                            let (start,end) = if let Some(tail) = range.strip_prefix('-') { (bytes.len().saturating_sub(tail.parse().unwrap()), bytes.len()-1) } else {
                                let (start,end) = range.split_once('-').unwrap(); (start.parse::<usize>().unwrap(), end.parse::<usize>().unwrap())
                            };
                            assert!(start <= end && end < bytes.len());
                            let part = &bytes[start..=end];
                            let status = if mode == 1 || mode == 11 && number > 1 { "200 OK" } else { "206 Partial Content" };
                            let etag = if mode == 4 { "" } else if mode == 3 && number > 1 { "ETag: \"changed\"\r\n" } else { "ETag: \"fixture\"\r\n" };
                            let encoding = if mode == 5 { "Content-Encoding: gzip\r\n" } else { "" };
                            let length = if mode == 9 { "Transfer-Encoding: chunked\r\n".into() } else { format!("Content-Length: {}\r\n", part.len()) };
                            let head = format!("HTTP/1.1 {status}\r\nContent-Range: bytes {}-{end}/{}\r\n{length}{etag}{encoding}Content-Type: application/zip\r\nConnection: close\r\n\r\n", start+usize::from(mode==2), bytes.len());
                            if socket.write_all(head.as_bytes()).await.is_err() { return; }
                            if mode == 9 {
                                let _ = socket.write_all(format!("{:x}\r\n", part.len()+1).as_bytes()).await;
                                let _ = socket.write_all(part).await; let _ = socket.write_all(b"x\r\n0\r\n\r\n").await;
                            } else { let count = part.len()-usize::from(mode==6); let _ = socket.write_all(&part[..count]).await; }
                        });
                    }
                    Some(result) = tasks.join_next(), if !tasks.is_empty() => { result.unwrap(); }
                }
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }
}
fn store() -> (tempfile::TempDir, Store) {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: root.path().join("data"),
        downloads: root.path().join("downloads"),
    })
    .unwrap();
    store
        .save_proxy_settings(
            &Settings::default(),
            Settings {
                mode: Mode::Direct,
                ..Default::default()
            },
            LoginChange::Remove,
        )
        .unwrap();
    (root, store)
}
fn job(store: &Store, url: &str) -> Job {
    store
        .new_download(url, None, None, Uuid::new_v4().to_string())
        .unwrap()
}

#[tokio::test]
async fn dotnet_archive_and_empty_archive_preview_without_queueing_or_extraction() {
    let (_root, store) = store();
    let server = Server::new(include_bytes!("fixtures/zip/dotnet.zip").to_vec(), 0).await;
    let preview =
        zip_preview::inspect(&store, &job(&store, &server.url), &CancellationToken::new())
            .await
            .unwrap();
    assert_eq!(preview.entries.len(), 3);
    assert!(preview.entries[0].directory);
    assert_eq!(preview.entries[2].name, "release/日本語.txt");
    assert_eq!(preview.expanded_size, 58);
    assert_eq!(server.requests.lock().unwrap().len(), 1);
    assert!(store.list().unwrap().is_empty());
    assert_eq!(
        std::fs::read_dir(&store.paths.downloads).unwrap().count(),
        0
    );
    let server = Server::new(archive(0, 0, false).bytes, 0).await;
    assert!(
        zip_preview::inspect(&store, &job(&store, &server.url), &CancellationToken::new())
            .await
            .unwrap()
            .entries
            .is_empty()
    );
}

#[tokio::test]
async fn large_directory_and_zip64_use_bounded_validated_range_reads() {
    let (_root, store) = store();
    for zip64 in [false, true] {
        let archive = archive(1800, 2 * 1024 * 1024, zip64);
        let archive_size = archive.bytes.len();
        let server = Server::new(archive.bytes, 0).await;
        let preview =
            zip_preview::inspect(&store, &job(&store, &server.url), &CancellationToken::new())
                .await
                .unwrap();
        assert_eq!(preview.entries.len(), 1800);
        assert_eq!(preview.entries[0].size, 2 * 1024 * 1024);
        assert_eq!(preview.archive_size, archive_size as u64);
        assert!(preview.metadata_bytes < 300_000);
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].to_lowercase().contains("if-range: \"fixture\""));
    }
}

#[tokio::test]
async fn incompatible_or_changed_servers_stop_without_full_download_fallback() {
    let (_root, store) = store();
    for mode in [1, 2, 3, 4, 5, 6, 7, 9, 11] {
        let server = Server::new(archive(1800, 200_000, false).bytes, mode).await;
        let error =
            zip_preview::inspect(&store, &job(&store, &server.url), &CancellationToken::new())
                .await
                .unwrap_err();
        assert!(!error.to_string().is_empty());
        assert!(server.requests.lock().unwrap().len() <= 2, "mode {mode}");
        assert!(store.list().unwrap().is_empty());
    }
}

#[tokio::test]
async fn cancellation_closes_a_stalled_preview() {
    let (_root, store) = store();
    let server = Server::new(archive(1, 10, false).bytes, 10).await;
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let store2 = store.clone();
    let job = job(&store, &server.url);
    let task = tokio::spawn(async move { zip_preview::inspect(&store2, &job, &token).await });
    tokio::time::timeout(Duration::from_secs(3), async {
        while server.requests.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    cancel.cancel();
    let result = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
    assert!(result.unwrap_err().to_string().contains("cancelled"));
}

#[tokio::test]
async fn malformed_directories_and_resource_limits_are_rejected() {
    let (_root, store) = store();
    for mutation in 0..9 {
        let mut archive = archive(2, 10, false);
        match mutation {
            0 => put16(&mut archive.bytes, archive.end + 4, 1),
            1 => put32(&mut archive.bytes, archive.end + 16, u32::MAX - 1),
            2 => put16(&mut archive.bytes, archive.central + 28, 4097),
            3 => put32(&mut archive.bytes, archive.central + 42, u32::MAX - 1),
            4 => put16(&mut archive.bytes, archive.central + 34, 1),
            5 => put16(&mut archive.bytes, archive.central + 8, 0x2000),
            6 => {
                put16(&mut archive.bytes, archive.end + 8, 10_001);
                put16(&mut archive.bytes, archive.end + 10, 10_001);
            }
            7 => put32(&mut archive.bytes, archive.end + 12, 4 * 1024 * 1024 + 1),
            8 => archive.bytes[archive.central + 46] = 0xff,
            _ => unreachable!(),
        }
        let server = Server::new(archive.bytes, 0).await;
        assert!(
            zip_preview::inspect(&store, &job(&store, &server.url), &CancellationToken::new())
                .await
                .is_err(),
            "mutation {mutation}"
        );
    }
}

#[tokio::test]
async fn encrypted_names_and_unusual_paths_are_display_only() {
    let (_root, store) = store();
    let mut archive = archive(1, 0, false);
    let name = b"../hidden\x00entry-00000.txt";
    assert_eq!(name.len(), 25);
    archive.bytes[archive.central + 46..archive.central + 46 + name.len()].copy_from_slice(name);
    put16(&mut archive.bytes, archive.central + 8, 0x801);
    let server = Server::new(archive.bytes, 0).await;
    let preview =
        zip_preview::inspect(&store, &job(&store, &server.url), &CancellationToken::new())
            .await
            .unwrap();
    assert!(preview.entries[0].encrypted && preview.entries[0].unusual_path);
    assert!(preview.entries[0].name.contains("\\u{0}") && !preview.entries[0].name.contains('\0'));
    assert_eq!(
        std::fs::read_dir(&store.paths.downloads).unwrap().count(),
        0
    );
}

#[tokio::test]
async fn preview_browser_preflight_and_download_share_the_custom_proxy() {
    let (_root, store) = store();
    let archive = archive(3, 1024 * 1024, false);
    let expected = archive.bytes.clone();
    let server = Server::new(archive.bytes, 0).await;
    let proxy = reqwest::Url::parse(&server.url)
        .unwrap()
        .origin()
        .ascii_serialization();
    store
        .save_proxy_settings(
            &store.proxy_settings().unwrap(),
            Settings {
                mode: Mode::Custom,
                endpoint: proxy,
                ..Default::default()
            },
            LoginChange::Remove,
        )
        .unwrap();
    let mut job = job(&store, "http://only-proxy.invalid/archive.zip");
    assert_eq!(
        zip_preview::inspect(&store, &job, &CancellationToken::new())
            .await
            .unwrap()
            .entries
            .len(),
        3
    );
    download_engine::capture::preflight(
        &store,
        &mut job,
        download_engine::capture::ExpectedFile::browser(expected.len() as u64, "application/zip")
            .unwrap(),
    )
    .await
    .unwrap();
    let job = store.enqueue(&job).unwrap();
    download_engine::engine::run(
        store.clone(),
        job.clone(),
        CancellationToken::new(),
        download_engine::bandwidth::Limiter::new(&Default::default()).unwrap(),
    )
    .await
    .unwrap();
    let saved = store.get(&job.id).unwrap().unwrap();
    assert_eq!(saved.status, Status::Complete, "{:?}", saved.error);
    assert_eq!(std::fs::read(saved.saved_path.unwrap()).unwrap(), expected);
    assert!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|r| r.starts_with("GET http://only-proxy.invalid/archive.zip "))
    );
}

#[cfg(windows)]
#[tokio::test]
async fn preview_reuses_only_the_selected_download_login_and_does_not_start_it() {
    use download_engine::{
        auth,
        scheduler::{Command, Scheduler},
    };
    let (_root, store) = store();
    store.set_queue_running("main", false).unwrap();
    let server = Server::new(archive(1, 10, false).bytes, 12).await;
    let mut job = job(&store, &server.url);
    job.status = Status::Paused;
    let job = store.enqueue(&job).unwrap();
    assert!(
        zip_preview::inspect(&store, &job, &CancellationToken::new())
            .await
            .unwrap_err()
            .to_string()
            .contains("401")
    );
    let scheduler = Scheduler::start(store.clone()).unwrap();
    scheduler
        .send(Command::Authentication(auth::Update {
            request: Uuid::new_v4(),
            job: job.id.clone(),
            expected_url: job.url.clone(),
            credentials: Some(auth::Credentials::basic("user".into(), "pass".into())),
            restart_confirmed: false,
            resume: false,
        }))
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(event) = scheduler.auth_events.try_recv() {
                break event;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(event.result.is_ok(), "{:?}", event.result);
    let authenticated = store.get(&job.id).unwrap().unwrap();
    let result = zip_preview::inspect(&store, &authenticated, &CancellationToken::new()).await;
    scheduler
        .send(Command::Authentication(auth::Update {
            request: Uuid::new_v4(),
            job: job.id.clone(),
            expected_url: job.url.clone(),
            credentials: None,
            restart_confirmed: false,
            resume: false,
        }))
        .unwrap();
    let cleanup = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(event) = scheduler.auth_events.try_recv() {
                break event;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(cleanup.result.is_ok());
    drop(scheduler);
    assert_eq!(result.unwrap().entries.len(), 1);
    assert_eq!(store.get(&job.id).unwrap().unwrap().status, Status::Paused);
}
