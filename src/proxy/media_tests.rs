use super::*;
use crate::{
    bandwidth::{Limiter, Limits},
    engine, hls,
    model::Status,
    paths::AppPaths,
    queue::MAIN_QUEUE,
    website_grabber,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_util::sync::CancellationToken;

struct ProxyFixture {
    _root: tempfile::TempDir,
    store: Store,
    requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}
impl ProxyFixture {
    async fn new(pages: HashMap<&'static str, (Vec<u8>, &'static str)>) -> Self {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(AppPaths {
            data: root.path().join("data"),
            downloads: root.path().join("files"),
        })
        .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        store
            .save_proxy_settings(
                &Settings::default(),
                Settings {
                    mode: Mode::Custom,
                    endpoint: format!("http://{}", listener.local_addr().unwrap()),
                    ..Default::default()
                },
                LoginChange::Replace(Login::new("proxy-fixture".into(), "proxy-secret".into())),
            )
            .unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let record = requests.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                while !bytes.ends_with(b"\r\n\r\n") {
                    bytes.push(socket.read_u8().await.unwrap());
                    assert!(bytes.len() < 65536);
                }
                let request = String::from_utf8(bytes).unwrap();
                let url = Url::parse(
                    request
                        .lines()
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .nth(1)
                        .unwrap(),
                )
                .unwrap();
                assert!(
                    request
                        .to_ascii_lowercase()
                        .contains("proxy-authorization: basic ")
                );
                assert!(
                    !request
                        .lines()
                        .any(|line| line.to_ascii_lowercase().starts_with("authorization:"))
                );
                record.lock().unwrap().push(url.path().to_string());
                if url.path() == "/outside" {
                    socket.write_all(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/private\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                } else {
                    let (body, mime) = pages.get(url.path()).expect("Unexpected proxy path");
                    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).as_bytes()).await.unwrap();
                    socket.write_all(body).await.unwrap();
                }
            }
        });
        Self {
            _root: root,
            store,
            requests,
            task,
        }
    }
    fn route(&self) -> Route {
        Route::load(&self.store).unwrap()
    }
}
impl Drop for ProxyFixture {
    fn drop(&mut self) {
        self.task.abort();
        let settings = self.store.proxy_settings().unwrap();
        self.store
            .save_proxy_settings(&settings, Settings::default(), LoginChange::Remove)
            .unwrap();
    }
}
#[tokio::test]
async fn hls_preview_and_complete_transfer_use_proxy_for_every_request() {
    let mut segment = vec![7u8; 188 * 4];
    for at in (0..segment.len()).step_by(188) {
        segment[at] = 0x47;
    }
    let fixture = ProxyFixture::new(HashMap::from([
        (
            "/master.m3u8",
            (
                b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=500000\nvideo.m3u8\n".to_vec(),
                "application/vnd.apple.mpegurl",
            ),
        ),
        (
            "/video.m3u8",
            (
                b"#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\nsegment.ts\n#EXT-X-ENDLIST\n"
                    .to_vec(),
                "application/vnd.apple.mpegurl",
            ),
        ),
        ("/segment.ts", (segment.clone(), "video/mp2t")),
    ]))
    .await;
    let cancel = CancellationToken::new();
    let inspection = hls::inspect_via("http://93.184.215.14/master.m3u8", &cancel, fixture.route())
        .await
        .unwrap();
    let preview = hls::preview_with_audio_via(&inspection, 0, 0, &cancel, fixture.route())
        .await
        .unwrap();
    assert!(
        !fixture
            .requests
            .lock()
            .unwrap()
            .contains(&"/segment.ts".into())
    );
    let job = fixture
        .store
        .enqueue_hls(
            &preview,
            "proxy-video.ts",
            None,
            MAIN_QUEUE,
            &uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
    let id = job.id.clone();
    engine::run(
        fixture.store.clone(),
        job,
        cancel,
        Limiter::new(&Limits::default()).unwrap(),
    )
    .await
    .unwrap();
    let complete = fixture.store.get(&id).unwrap().unwrap();
    assert_eq!(complete.status, Status::Complete, "{:?}", complete.error);
    assert_eq!(
        std::fs::read(complete.saved_path.unwrap()).unwrap(),
        segment
    );
    assert_eq!(
        *fixture.requests.lock().unwrap(),
        ["/master.m3u8", "/video.m3u8", "/video.m3u8", "/segment.ts"]
    );
}
#[tokio::test]
async fn dash_and_grabber_use_proxy_and_keep_destination_boundaries() {
    let fixture=ProxyFixture::new(HashMap::from([
        ("/main.mpd",(br#"<MPD type="static" mediaPresentationDuration="PT2S"><Period><AdaptationSet mimeType="video/mp4"><Representation id="v" bandwidth="500000"><SegmentTemplate duration="2" initialization="init.mp4" media="part-$Number$.m4s"/></Representation></AdaptationSet></Period></MPD>"#.to_vec(),"application/dash+xml")),
        ("/init.mp4",(b"initialization".to_vec(),"video/mp4")),
        ("/part-1.m4s",(b"segment".to_vec(),"video/mp4")),
        ("/",(br#"<a href="file.zip">file</a><a href="outside">outside</a>"#.to_vec(),"text/html"))
    ])).await;
    let cancel = CancellationToken::new();
    let inspection = hls::inspect_via("http://93.184.215.14/main.mpd", &cancel, fixture.route())
        .await
        .unwrap();
    let preview = hls::preview_with_audio_via(&inspection, 0, 0, &cancel, fixture.route())
        .await
        .unwrap();
    let mut network = hls::network::Network::via(&preview.source, fixture.route())
        .await
        .unwrap();
    for part in &preview.parts {
        assert!(
            !network
                .get(&part.url, None, &cancel)
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap()
                .is_empty()
        );
    }
    let config = website_grabber::Config::new("http://93.184.215.14/", "zip", 1, 10)
        .unwrap()
        .with_proxy(&fixture.store)
        .unwrap();
    let report = website_grabber::scan(config, cancel.clone(), |_| {})
        .await
        .unwrap();
    assert_eq!(report.files.len(), 1);
    assert_eq!(report.pages_failed, 1);
    assert!(fixture.store.list().unwrap().is_empty());
    let error = network
        .get("http://93.184.215.14/outside", None, &cancel)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("different local"));
    assert!(
        !fixture
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|p| p == "/private")
    );
}
#[tokio::test]
async fn a_failed_custom_proxy_does_not_contact_the_origin() {
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", proxy.local_addr().unwrap());
    drop(proxy);
    let route = Route {
        settings: Settings {
            mode: Mode::Custom,
            endpoint,
            ..Default::default()
        },
        login: None,
    };
    let url = format!("http://{}/index.m3u8", origin.local_addr().unwrap());
    assert!(
        hls::inspect_via(&url, &CancellationToken::new(), route)
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(200), origin.accept())
            .await
            .is_err()
    );
}
