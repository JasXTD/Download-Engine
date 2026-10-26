use download_engine::{
    bandwidth::{Limiter, Limits},
    engine, hls,
    model::{Job, Status},
    paths::AppPaths,
    queue::MAIN_QUEUE,
    store::Store,
};
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

#[path = "hls/activity.rs"]
mod activity;
#[path = "hls/dash.rs"]
mod dash;
#[path = "hls/merge.rs"]
mod merge;
#[path = "hls/sidx.rs"]
mod sidx;

#[derive(Clone)]
struct Reply {
    body: Vec<u8>,
    headers: String,
    status: u16,
    cut: bool,
    ignore_range: bool,
    headers_for_start: Option<(usize, String)>,
}
impl Reply {
    fn bytes(body: impl AsRef<[u8]>) -> Self {
        Self {
            body: body.as_ref().into(),
            headers: String::new(),
            status: 200,
            cut: false,
            ignore_range: false,
            headers_for_start: None,
        }
    }
    fn redirect(location: &str) -> Self {
        Self {
            status: 302,
            headers: format!("Location: {location}\r\n"),
            ..Self::bytes([])
        }
    }
}
struct PauseRequest {
    target: String,
    range: Option<String>,
    cancel: CancellationToken,
}
struct Server {
    base: String,
    routes: Arc<Mutex<HashMap<String, Reply>>>,
    requests: Arc<Mutex<Vec<String>>>,
    pause: Arc<Mutex<Option<PauseRequest>>>,
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
        let pause = Arc::new(Mutex::new(None::<PauseRequest>));
        let (r, q, p) = (routes.clone(), requests.clone(), pause.clone());
        let task = tokio::spawn(async move {
            let mut peers = JoinSet::new();
            loop {
                tokio::select! {
                    connection = listener.accept() => {
                        let Ok((mut socket,_)) = connection else { break };
                        let (routes,requests,pause) = (r.clone(),q.clone(),p.clone());
                        peers.spawn(async move {
                            let mut data = Vec::new(); let mut buffer = [0;1024];
                            while !data.windows(4).any(|v| v == b"\r\n\r\n") {
                                let Ok(n) = socket.read(&mut buffer).await else {return};
                                if n == 0 || data.len()+n > 32768 { return; }
                                data.extend_from_slice(&buffer[..n]);
                            }
                            let request = String::from_utf8(data).unwrap();
                            let path = request.lines().next().unwrap().split_whitespace().nth(1).unwrap();
                            requests.lock().unwrap().push(request.clone());
                            let range = request.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("range: bytes=").map(str::to_owned));
                            let cancellation = {
                                let mut pending = pause.lock().unwrap();
                                if pending.as_ref().is_some_and(|p| p.target == path && p.range == range) {
                                    pending.take()
                                } else { None }
                            };
                            if let Some(pause) = cancellation {
                                pause.cancel.cancel();
                                return;
                            }
                            let mut reply = routes.lock().unwrap().get(path).cloned().unwrap_or(Reply {status:404,..Reply::bytes("missing")});
                            let total = reply.body.len();
                            if !reply.ignore_range && reply.status == 200 && let Some(range) = range {
                                let (start,end) = range.split_once('-').unwrap(); let (start,end): (usize,usize) = (start.parse().unwrap(),end.parse().unwrap());
                                if end < total && start <= end {
                                    if let Some((offset,headers))=&reply.headers_for_start && *offset == start { reply.headers=headers.clone(); }
                                    reply.body = reply.body[start..=end].to_vec(); reply.status = 206;
                                    reply.headers.push_str(&format!("Content-Range: bytes {start}-{end}/{total}\r\n"));
                                } else { reply.status = 416; reply.body.clear(); }
                            }
                            let header = format!("HTTP/1.1 {} fixture\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n",reply.status,reply.body.len(),reply.headers);
                            if socket.write_all(header.as_bytes()).await.is_err() {return;}
                            let body = if reply.cut { &reply.body[..reply.body.len()/2] } else { &reply.body };
                            for chunk in body.chunks(18800) {
                                if socket.write_all(chunk).await.is_err() {return;}
                            }
                        });
                    },
                    _=peers.join_next(), if !peers.is_empty()=>{},
                }
            }
        });
        Self {
            base,
            routes,
            requests,
            pause,
            task,
        }
    }
    fn set(&self, path: &str, reply: Reply) {
        self.routes.lock().unwrap().insert(path.into(), reply);
    }
    fn pause_on(&self, part: &hls::Part) -> CancellationToken {
        let url = reqwest::Url::parse(&part.url).unwrap();
        assert_eq!(url.origin().ascii_serialization(), self.base);
        let mut target = url.path().to_owned();
        if let Some(query) = url.query() {
            target.push('?');
            target.push_str(query);
        }
        assert!(self.routes.lock().unwrap().contains_key(&target));
        let cancel = CancellationToken::new();
        let mut pending = self.pause.lock().unwrap();
        assert!(pending.is_none(), "A pause request is already pending");
        *pending = Some(PauseRequest {
            target,
            range: part
                .range
                .as_ref()
                .map(|r| format!("{}-{}", r.start, r.start + r.length - 1)),
            cancel: cancel.clone(),
        });
        cancel
    }
    fn count(&self, path: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.starts_with(&format!("GET {path} HTTP/")))
            .count()
    }
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }
}
fn fixture() -> (tempfile::TempDir, Store) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    (temp, store)
}
fn ts(packets: usize, value: u8) -> Vec<u8> {
    let mut data = vec![value; 188 * packets];
    for packet in data.chunks_mut(188) {
        packet[0] = 0x47;
    }
    data
}
fn media(parts: &[&str]) -> String {
    format!(
        "#EXTM3U\n#EXT-X-TARGETDURATION:6\n{}#EXT-X-ENDLIST\n",
        parts
            .iter()
            .map(|url| format!("#EXTINF:6,\n{url}\n"))
            .collect::<String>()
    )
}
async fn playlist(server: &Server, path: &str) -> hls::Playlist {
    hls::inspect(&server.url(path), &CancellationToken::new())
        .await
        .unwrap()
        .playlist
        .unwrap()
}
fn enqueue(store: &Store, playlist: &hls::Playlist) -> Job {
    store
        .enqueue_hls(
            playlist,
            &format!("video.{}", playlist.output_extension()),
            None,
            MAIN_QUEUE,
            &uuid::Uuid::new_v4().to_string(),
        )
        .unwrap()
}
async fn run(store: &Store, job: Job) -> Job {
    let id = job.id.clone();
    engine::run(
        store.clone(),
        job,
        CancellationToken::new(),
        Limiter::new(&Limits::default()).unwrap(),
    )
    .await
    .unwrap();
    store.get(&id).unwrap().unwrap()
}

#[tokio::test]
async fn reader_security_returned_streams_never_contact_local_roots() {
    let server = Server::new().await;
    server.set("/video.m3u8", Reply::bytes(media(&["part.ts"])));
    let cancel = CancellationToken::new();
    for url in [
        server.url("/video.m3u8"),
        server.url("/video.mpd"),
        server.url("/video.m3u8").replace("127.0.0.1", "localhost"),
        "http://[::1]/video.m3u8".into(),
        "http://192.168.1.1/video.m3u8".into(),
    ] {
        let reading = download_engine::script_automation::Reading::from_value(serde_json::json!({
            "items": [{"kind": "hls", "url": url}]
        }))
        .unwrap();
        let error = hls::inspect_reader_via(
            &reading.single_stream().unwrap().url,
            "https://example.com/watch",
            &cancel,
            download_engine::proxy::Route::direct(),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("public internet"), "{error:#}");
    }
    assert!(server.requests.lock().unwrap().is_empty());
    // An explicitly entered local stream remains usable.
    let ordinary = hls::inspect(&server.url("/video.m3u8"), &cancel)
        .await
        .unwrap();
    assert!(!ordinary.public_only && !ordinary.playlist.unwrap().public_only);
    assert_eq!(server.count("/video.m3u8"), 1);
}

#[tokio::test]
async fn reader_security_quality_selection_and_saved_resume_keep_the_boundary() {
    let server = Server::new().await;
    server.set(
        "/master.m3u8",
        Reply::bytes("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000000\nvideo.m3u8\n"),
    );
    server.set("/video.m3u8", Reply::bytes(media(&["part.ts"])));
    let cancel = CancellationToken::new();
    let mut inspection = hls::inspect(&server.url("/master.m3u8"), &cancel)
        .await
        .unwrap();
    inspection.public_only = true;
    let error = hls::preview(&inspection, 0, &cancel).await.unwrap_err();
    assert!(error.to_string().contains("public internet"), "{error:#}");
    assert_eq!(server.count("/video.m3u8"), 0);

    let mut selected = playlist(&server, "/video.m3u8").await;
    let legacy = serde_json::to_string(&selected).unwrap();
    assert!(!legacy.contains("public_only"));
    assert!(
        !serde_json::from_str::<hls::Playlist>(&legacy)
            .unwrap()
            .public_only
    );
    selected.public_only = true;
    let (_temp, store) = fixture();
    store
        .save_proxy_settings(
            &store.proxy_settings().unwrap(),
            download_engine::proxy::Settings {
                mode: download_engine::proxy::Mode::Direct,
                ..Default::default()
            },
            download_engine::proxy::LoginChange::Remove,
        )
        .unwrap();
    let job = enqueue(&store, &selected);
    let reopened = Store::open(store.paths.clone()).unwrap();
    let job = reopened.get(&job.id).unwrap().unwrap();
    assert!(job.public_only && job.hls.as_ref().unwrap().playlist.public_only);
    server.requests.lock().unwrap().clear();
    let failed = run(&reopened, job).await;
    assert_eq!(failed.status, Status::Failed);
    assert!(failed.error.unwrap().contains("public internet"));
    assert!(server.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn reader_security_selected_files_keep_the_boundary_after_reopening() {
    let server = Server::new().await;
    let (_temp, store) = fixture();
    store
        .save_proxy_settings(
            &store.proxy_settings().unwrap(),
            download_engine::proxy::Settings {
                mode: download_engine::proxy::Mode::Direct,
                ..Default::default()
            },
            download_engine::proxy::LoginChange::Remove,
        )
        .unwrap();
    let reading = download_engine::script_automation::Reading::from_value(serde_json::json!({
        "items": [{"kind": "file", "url": server.url("/private.mp4")}]
    }))
    .unwrap();
    let review = reading.review("https://example.com/watch").unwrap();
    assert!(review.public_only);
    let receipt = format!("video:{}", uuid::Uuid::new_v4());
    store
        .request_browser_video_review(&receipt, &review)
        .unwrap();
    let reopened = Store::open(store.paths.clone()).unwrap();
    assert!(
        reopened
            .next_browser_video_review()
            .unwrap()
            .unwrap()
            .review
            .public_only
    );
    let ids = reopened
        .add_browser_video_selection(
            &receipt,
            &[0],
            MAIN_QUEUE,
            download_engine::browser_batch::Destination::Video,
        )
        .unwrap();
    let reopened = Store::open(store.paths.clone()).unwrap();
    let job = reopened.get(&ids[0]).unwrap().unwrap();
    assert!(job.public_only);
    let failed = run(&reopened, job).await;
    assert_eq!(failed.status, Status::Failed);
    assert!(failed.error.unwrap().contains("public internet"));
    assert!(server.requests.lock().unwrap().is_empty());
}

async fn pause_at_part(server: &Server, store: &Store, job: Job, part: &hls::Part) -> Job {
    // Cancel once at the exact request, before any response bytes. Polling a
    // slowed response races completion and counts helper verification time
    // against the pause deadline. Range matching keeps index/init reads intact.
    let cancel = server.pause_on(part);
    let id = job.id.clone();
    let worker = engine::run(
        store.clone(),
        job,
        cancel.clone(),
        Limiter::new(&Limits::default()).unwrap(),
    );
    tokio::pin!(worker);
    let result = tokio::select! {
        result = &mut worker => result,
        _ = cancel.cancelled() => {
            // Bound cancellation handling after the actual request, not startup.
            tokio::time::timeout(Duration::from_secs(8), worker)
                .await
                .expect("Media worker did not stop after cancellation")
        }
    };
    result.unwrap();
    let paused = store.get(&id).unwrap().unwrap();
    assert!(
        cancel.is_cancelled(),
        "Download did not reach {part:?}: {:?}, {:?}",
        paused.status,
        paused.error
    );
    assert_eq!(paused.status, Status::Paused, "{:?}", paused.error);
    paused
}

#[tokio::test]
async fn quality_preview_fetches_no_segments_or_jobs_until_reviewed_and_downloads_exact_selected_bytes()
 {
    let server = Server::new().await;
    let (_temp, store) = fixture();
    server.set("/master.m3u8",Reply::bytes("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000000,RESOLUTION=640x360\nlow.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=4000000,RESOLUTION=1280x720\nhigh.m3u8\n"));
    server.set(
        "/high.m3u8",
        Reply::bytes(media(&["a.ts?token=keep", "b.ts"])),
    );
    let a = ts(12, 1);
    let b = ts(10, 2);
    server.set("/a.ts?token=keep", Reply::bytes(&a));
    server.set("/b.ts", Reply::bytes(&b));
    let inspection = hls::inspect(&server.url("/master.m3u8"), &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(inspection.variants[0].resolution, Some((1280, 720)));
    let preview = hls::preview(&inspection, 0, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(preview.media_count(), 2);
    assert_eq!(preview.duration_ms, 12000);
    assert!(store.list().unwrap().is_empty());
    assert_eq!(server.requests.lock().unwrap().len(), 2);
    let job = enqueue(&store, &preview);
    assert_eq!(job.category(), download_engine::categories::Category::Video);
    let saved = run(&store, job).await;
    assert_eq!(saved.status, Status::Complete, "{:?}", saved.error);
    let expected = [a, b].concat();
    assert_eq!(std::fs::read(saved.saved_path.unwrap()).unwrap(), expected);
    assert!(saved.integrity.actual.is_some());
    assert_eq!(saved.downloaded, expected.len() as u64);
    assert_eq!(server.count("/low.m3u8"), 0);
    for request in server.requests.lock().unwrap().iter() {
        let lower = request.to_ascii_lowercase();
        assert!(
            !lower.contains("cookie:")
                && !lower.contains("authorization:")
                && !lower.contains("referer:")
        );
    }
}

#[tokio::test]
async fn streams_found_on_a_page_send_only_its_origin_as_referer() {
    let server = Server::new().await;
    let (_temp, store) = fixture();
    server.set(
        "/master.m3u8",
        Reply::bytes("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000000,RESOLUTION=640x360\nlow.m3u8\n"),
    );
    server.set("/low.m3u8", Reply::bytes(media(&["a.ts", "b.ts"])));
    server.set("/a.ts", Reply::bytes(ts(4, 1)));
    server.set("/b.ts", Reply::bytes(ts(4, 2)));
    let page = "http://videos.example.test/watch/1?session=secret";
    let cancel = CancellationToken::new();
    let inspection = hls::inspect_page_via(
        &server.url("/master.m3u8"),
        Some(page),
        &cancel,
        download_engine::proxy::Route::direct(),
    )
    .await
    .unwrap();
    assert_eq!(inspection.page.as_deref(), Some(page));
    let preview = hls::preview(&inspection, 0, &cancel).await.unwrap();
    let job = store
        .enqueue_found_hls(
            &preview,
            Some(page),
            "video.ts",
            None,
            MAIN_QUEUE,
            "found-on-page",
        )
        .unwrap();
    assert_eq!(job.source_page.as_deref(), Some(page));
    let saved = run(&store, job).await;
    assert_eq!(saved.status, Status::Complete, "{:?}", saved.error);
    // Inspection, preview, the recheck before downloading and every segment.
    let requests = server.requests.lock().unwrap().clone();
    assert!(requests.len() >= 5, "{}", requests.len());
    for request in &requests {
        let lower = request.to_ascii_lowercase();
        assert!(
            lower.contains("\r\nreferer: http://videos.example.test/\r\n"),
            "{request}"
        );
        assert!(!lower.contains("session=secret") && !lower.contains("cookie:"));
    }
    // An HTTPS page never sends its address to a plain HTTP server.
    server.requests.lock().unwrap().clear();
    hls::inspect_page_via(
        &server.url("/master.m3u8"),
        Some("https://videos.example.test/watch/1"),
        &cancel,
        download_engine::proxy::Route::direct(),
    )
    .await
    .unwrap();
    let requests = server.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    assert!(!requests[0].to_ascii_lowercase().contains("referer:"));
}

#[tokio::test]
async fn pause_reload_and_resume_reuses_only_complete_verified_segments() {
    let server = Server::new().await;
    let (_temp, store) = fixture();
    let a = ts(100, 3);
    let b = ts(1000, 4);
    server.set("/index.m3u8", Reply::bytes(media(&["a.ts", "b.ts"])));
    server.set("/a.ts", Reply::bytes(&a));
    server.set("/b.ts", Reply::bytes(&b));
    let preview = playlist(&server, "/index.m3u8").await;
    let paused = pause_at_part(
        &server,
        &store,
        enqueue(&store, &preview),
        &preview.parts[1],
    )
    .await;
    let reloaded = Store::open(store.paths.clone()).unwrap();
    let paused = reloaded.get(&paused.id).unwrap().unwrap();
    assert_eq!(paused.status, Status::Paused);
    assert_eq!(paused.hls.as_ref().unwrap().completed.len(), 1);
    assert!(paused.saved_path.is_none());
    let saved = run(&reloaded, paused).await;
    assert_eq!(saved.status, Status::Complete, "{:?}", saved.error);
    assert_eq!(
        std::fs::read(saved.saved_path.unwrap()).unwrap(),
        [a, b].concat()
    );
    assert_eq!(server.count("/a.ts"), 1);
    assert_eq!(server.count("/b.ts"), 2);
}

#[tokio::test]
async fn changed_or_newly_encrypted_playlists_are_rejected_before_media_or_keys() {
    let server = Server::new().await;
    let (_temp, store) = fixture();
    server.set("/index.m3u8", Reply::bytes(media(&["a.ts"])));
    let preview = playlist(&server, "/index.m3u8").await;
    server.set("/index.m3u8", Reply::bytes(media(&["other.ts"])));
    let saved = run(&store, enqueue(&store, &preview)).await;
    assert_eq!(saved.status, Status::Failed);
    assert!(saved.error.unwrap().contains("changed"));
    server.set("/index.m3u8",Reply::bytes("#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXT-X-KEY:METHOD=AES-128,URI=\"key\"\n#EXTINF:6,\na.ts\n#EXT-X-ENDLIST\n"));
    let saved = run(&store, enqueue(&store, &preview)).await;
    assert_eq!(saved.status, Status::Failed);
    assert!(saved.error.unwrap().contains("Encrypted"));
    assert_eq!(
        server.count("/a.ts") + server.count("/other.ts") + server.count("/key"),
        0
    );
}

fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    [
        (8u32 + body.len() as u32).to_be_bytes().to_vec(),
        kind.to_vec(),
        body.to_vec(),
    ]
    .concat()
}
#[tokio::test]
async fn fmp4_init_and_byte_ranges_produce_exact_ordered_output_and_reject_ignored_ranges() {
    let server = Server::new().await;
    let (_temp, store) = fixture();
    // Keep the synthetic byte-range fixture structurally valid as metadata is
    // now inspected for encrypted sample entries before publication.
    let init = [
        mp4_box(b"ftyp", b"iso6"),
        mp4_box(b"moov", &mp4_box(b"free", b"test")),
    ]
    .concat();
    let part = [mp4_box(b"moof", b"test"), mp4_box(b"mdat", b"payload")].concat();
    let all = [init.clone(), part.clone(), part.clone()].concat();
    let manifest = format!(
        "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXT-X-MAP:URI=\"all.mp4\",BYTERANGE=\"{}@0\"\n#EXTINF:6,\n#EXT-X-BYTERANGE:{}@{}\nall.mp4\n#EXTINF:6,\n#EXT-X-BYTERANGE:{}\nall.mp4\n#EXT-X-ENDLIST\n",
        init.len(),
        part.len(),
        init.len(),
        part.len()
    );
    server.set("/index.m3u8", Reply::bytes(manifest));
    server.set("/all.mp4", Reply::bytes(&all));
    let preview = playlist(&server, "/index.m3u8").await;
    let saved = run(&store, enqueue(&store, &preview)).await;
    assert_eq!(saved.status, Status::Complete, "{:?}", saved.error);
    assert_eq!(std::fs::read(saved.saved_path.unwrap()).unwrap(), all);
    server.set(
        "/all.mp4",
        Reply {
            ignore_range: true,
            ..Reply::bytes(&all)
        },
    );
    let failed = run(&store, enqueue(&store, &preview)).await;
    assert_eq!(failed.status, Status::Failed);
    assert!(failed.error.unwrap().contains("ignored"));
    assert!(failed.saved_path.is_none());
}

#[tokio::test]
async fn untrusted_playlists_cannot_fetch_another_local_origin_through_resources_or_redirects() {
    let server = Server::new().await;
    let other = Server::new().await;
    let (_temp, store) = fixture();
    server.set("/index.m3u8", Reply::bytes(media(&[&other.url("/secret")])));
    let preview = playlist(&server, "/index.m3u8").await;
    let saved = run(&store, enqueue(&store, &preview)).await;
    assert_eq!(saved.status, Status::Failed);
    assert!(saved.error.unwrap().contains("different local"));
    server.set("/redirect.m3u8", Reply::redirect(&other.url("/secret")));
    assert!(
        hls::inspect(&server.url("/redirect.m3u8"), &CancellationToken::new())
            .await
            .is_err()
    );
    assert!(other.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn truncated_segments_retry_without_ever_publishing_or_reusing_partial_bytes() {
    let server = Server::new().await;
    let (_temp, store) = fixture();
    let bytes = ts(300, 7);
    server.set("/index.m3u8", Reply::bytes(media(&["a.ts"])));
    server.set(
        "/a.ts",
        Reply {
            cut: true,
            ..Reply::bytes(&bytes)
        },
    );
    let first = run(
        &store,
        enqueue(&store, &playlist(&server, "/index.m3u8").await),
    )
    .await;
    assert_eq!(first.status, Status::Queued, "{:?}", first.error);
    assert!(first.retry.next_at.is_some());
    assert!(first.hls.as_ref().unwrap().completed.is_empty());
    assert!(first.saved_path.is_none());
    server.set("/a.ts", Reply::bytes(&bytes));
    let saved = run(&store, first).await;
    assert_eq!(saved.status, Status::Complete, "{:?}", saved.error);
    assert_eq!(std::fs::read(saved.saved_path.unwrap()).unwrap(), bytes);
}

#[tokio::test]
async fn invalid_container_and_unsafe_saved_progress_fail_closed() {
    let server = Server::new().await;
    let (_temp, store) = fixture();
    server.set("/index.m3u8", Reply::bytes(media(&["a.ts"])));
    server.set("/a.ts", Reply::bytes("<html>login required</html>"));
    let preview = playlist(&server, "/index.m3u8").await;
    let saved = run(&store, enqueue(&store, &preview)).await;
    assert_eq!(saved.status, Status::Failed);
    assert!(saved.saved_path.is_none());
    let mut corrupt = saved;
    corrupt.hls.as_mut().unwrap().playlist.parts[0].url = "file:///C:/private".into();
    assert!(store.save(&corrupt).is_err());
}

#[tokio::test]
async fn queue_scheduler_waits_for_user_enqueue_then_uses_normal_completion() {
    let server = Server::new().await;
    let (_temp, store) = fixture();
    store.set_queue_running(MAIN_QUEUE, false).unwrap();
    let bytes = ts(10, 6);
    server.set("/index.m3u8", Reply::bytes(media(&["a.ts"])));
    server.set("/a.ts", Reply::bytes(&bytes));
    let scheduler = download_engine::scheduler::Scheduler::start(store.clone()).unwrap();
    let preview = playlist(&server, "/index.m3u8").await;
    assert!(store.list().unwrap().is_empty());
    assert_eq!(server.count("/a.ts"), 0);
    let job = enqueue(&store, &preview);
    assert!(
        store.queues().unwrap()[0].running,
        "Adding a reviewed video must wake an idle Main queue"
    );
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let saved = store.get(&job.id).unwrap().unwrap();
            if saved.status == Status::Complete {
                break;
            }
            assert_ne!(saved.status, Status::Failed, "{:?}", saved.error);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let saved = store.get(&job.id).unwrap().unwrap();
    assert_eq!(std::fs::read(saved.saved_path.unwrap()).unwrap(), bytes);
    drop(scheduler);
}

#[tokio::test]
async fn altered_completed_segments_are_rejected_on_resume_and_restart_discards_them() {
    let server = Server::new().await;
    let (_temp, store) = fixture();
    let bytes = ts(100, 8);
    server.set("/index.m3u8", Reply::bytes(media(&["a.ts", "b.ts"])));
    server.set("/a.ts", Reply::bytes(&bytes));
    server.set(
        "/b.ts",
        Reply {
            status: 503,
            ..Reply::bytes("temporary")
        },
    );
    let saved = run(
        &store,
        enqueue(&store, &playlist(&server, "/index.m3u8").await),
    )
    .await;
    assert_eq!(saved.hls.as_ref().unwrap().completed.len(), 1);
    assert_eq!(saved.status, Status::Queued);
    let mut corrupt = bytes.clone();
    corrupt[1] ^= 1;
    std::fs::write(
        store.paths.parts(&saved.id).unwrap().join("hls-0.part"),
        corrupt,
    )
    .unwrap();
    let failed = run(&store, saved).await;
    assert_eq!(failed.status, Status::Failed);
    assert!(failed.error.as_deref().unwrap().contains("segment changed"));
    assert!(failed.saved_path.is_none());
    let now = chrono::Utc::now().timestamp();
    store.schedule_queue(MAIN_QUEUE, now + 3600, now).unwrap();
    let scheduler = download_engine::scheduler::Scheduler::start(store.clone()).unwrap();
    scheduler
        .send(download_engine::scheduler::Command::Restart(
            failed.id.clone(),
        ))
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !store
            .get(&failed.id)
            .unwrap()
            .unwrap()
            .hls
            .as_ref()
            .unwrap()
            .completed
            .is_empty()
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    drop(scheduler);
    server.set("/b.ts", Reply::bytes(&bytes));
    let saved = run(&store, store.get(&failed.id).unwrap().unwrap()).await;
    assert_eq!(saved.status, Status::Complete, "{:?}", saved.error);
    assert_eq!(
        std::fs::read(saved.saved_path.unwrap()).unwrap(),
        [bytes.clone(), bytes].concat()
    );
}

#[tokio::test]
async fn hls_payload_obeys_shared_quota_and_restarts_only_the_incomplete_segment() {
    let server = Server::new().await;
    let (_temp, store) = fixture();
    let first = ts(100, 2);
    let large = ts(12000, 3);
    server.set("/index.m3u8", Reply::bytes(media(&["a.ts", "b.ts"])));
    server.set("/a.ts", Reply::bytes(&first));
    server.set("/b.ts", Reply::bytes(&large));
    let limiter = Limiter::new(&Limits::default()).unwrap();
    store
        .set_quota_limits_at(
            download_engine::quota::Limits {
                hourly: Some(1024 * 1024),
                daily: None,
            },
            chrono::Utc::now().timestamp(),
        )
        .unwrap();
    let job = enqueue(&store, &playlist(&server, "/index.m3u8").await);
    let id = job.id.clone();
    engine::run(
        store.clone(),
        job,
        CancellationToken::new(),
        limiter.clone(),
    )
    .await
    .unwrap();
    let held = store.get(&id).unwrap().unwrap();
    assert_eq!(held.status, Status::Queued, "{:?}", held.error);
    assert!(held.quota_waiting);
    assert!(held.saved_path.is_none());
    assert_eq!(held.hls.as_ref().unwrap().completed.len(), 1);
    assert!(store.quota_status().unwrap().hourly_used >= 1024 * 1024);
    store
        .set_quota_limits_at(
            download_engine::quota::Limits::default(),
            chrono::Utc::now().timestamp(),
        )
        .unwrap();
    let saved = run(&store, held).await;
    assert_eq!(saved.status, Status::Complete, "{:?}", saved.error);
    assert_eq!(server.count("/a.ts"), 1);
    assert_eq!(
        std::fs::read(saved.saved_path.unwrap()).unwrap(),
        [first, large].concat()
    );
}

#[tokio::test]
async fn duplicate_receipts_cannot_change_quality_or_turn_a_playlist_into_a_direct_file() {
    let server = Server::new().await;
    let (_temp, store) = fixture();
    server.set("/index.m3u8", Reply::bytes(media(&["a.ts"])));
    let preview = playlist(&server, "/index.m3u8").await;
    let id = uuid::Uuid::new_v4().to_string();
    let first = store
        .enqueue_hls(&preview, "video.ts", None, MAIN_QUEUE, &id)
        .unwrap();
    let second = store
        .enqueue_hls(&preview, "video.ts", None, MAIN_QUEUE, &id)
        .unwrap();
    assert_eq!(first.id, second.id);
    let mut changed = preview;
    changed.parts[0].url = server.url("/different.ts");
    assert!(
        store
            .enqueue_hls(&changed, "video.ts", None, MAIN_QUEUE, &id)
            .is_err()
    );
    let raw = Job::new(
        &server.url("/index.m3u8"),
        None,
        store.paths.downloads.clone(),
        "direct-playlist".into(),
    )
    .unwrap();
    assert!(store.enqueue(&raw).is_err());
    assert_eq!(store.list().unwrap().len(), 1);
}
