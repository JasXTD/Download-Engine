//! Harmless local HTTP fixtures. Never compiled into production or Windows tests.
use super::*;
use crate::{
    session::{AddOptions, Session, SessionOptions},
    testkit::*,
};
use parking_lot::Mutex;
use std::collections::BTreeMap;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinSet,
    time::timeout,
};
use tokio_util::sync::CancellationToken;

type Handler = Arc<dyn Fn(&str, &str) -> (String, Vec<u8>) + Send + Sync>;
struct Server {
    base: String,
    requests: Arc<Mutex<Vec<(String, String)>>>,
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}
impl Server {
    async fn start(handler: Handler, slow: bool) -> Self {
        let socket = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", socket.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let cancel = CancellationToken::new();
        let task = tokio::spawn({
            let cancel = cancel.clone();
            let requests = requests.clone();
            async move {
                let mut clients = JoinSet::new();
                loop {
                    tokio::select! { biased;
                        _=cancel.cancelled()=>break,
                        Some(_)=clients.join_next(),if !clients.is_empty()=>{},
                        connection=socket.accept()=>{
                            let (mut stream,_)=connection.unwrap();let handler=handler.clone();let requests=requests.clone();
                            clients.spawn(async move {
                                let mut bytes=Vec::new();
                                while !bytes.ends_with(b"\r\n\r\n") {
                                    let mut one=[0]; if stream.read_exact(&mut one).await.is_err(){return;} bytes.push(one[0]); assert!(bytes.len()<8192);
                                }
                                let text=String::from_utf8(bytes).unwrap();
                                let target=text.split_whitespace().nth(1).unwrap();
                                let range=text.lines().find_map(|line|line.split_once(':').filter(|(name,_)|name.eq_ignore_ascii_case("range")).map(|(_,v)|v.trim())).unwrap_or("");
                                requests.lock().push((target.into(),range.into()));
                                let (header,body)=handler(target,range);
                                if stream.write_all(header.as_bytes()).await.is_err(){return;}
                                for chunk in body.chunks(16*1024) {
                                    if stream.write_all(chunk).await.is_err(){return;}
                                    if slow {tokio::time::sleep(Duration::from_millis(2)).await;}
                                }
                            });
                        }
                    }
                }
                clients.abort_all();
                while clients.join_next().await.is_some() {}
            }
        });
        Self {
            base,
            requests,
            cancel,
            task,
        }
    }
    async fn stop(self) {
        self.cancel.cancel();
        self.task.await.unwrap();
    }
}
fn response(status: &str, extra: &str, body: Vec<u8>) -> (String, Vec<u8>) {
    (
        format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n",
            body.len()
        ),
        body,
    )
}
fn range_response(data: &[u8], range: &str) -> (String, Vec<u8>) {
    let range = range.strip_prefix("bytes=").unwrap();
    let (a, b) = range.split_once('-').unwrap();
    let a: usize = a.parse().unwrap();
    let b: usize = b.parse().unwrap();
    assert!(a <= b && b < data.len());
    response(
        "206 Partial Content",
        &format!("Content-Range: bytes {a}-{b}/{}\r\n", data.len()),
        data[a..=b].to_vec(),
    )
}
async fn session(state: std::path::PathBuf) -> Arc<Session> {
    Session::new(SessionOptions {
        web_seeds: true,
        allow_loopback_peers: true,
        state_dir: Some(state),
        ..Default::default()
    })
    .await
    .unwrap()
}
async fn wait_for(session: &Session, check: impl Fn(&crate::torrent::TorrentStats) -> bool) {
    timeout(Duration::from_secs(30), async {
        loop {
            let snapshot = session.stats();
            let job = &snapshot.torrents["download"];
            assert!(job.error.is_none(), "{:?}", job.error);
            if check(job) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn range_seed_pause_stop_all_restart_and_verified_recovery() {
    let data = Arc::new(sample_bytes(8 * 1024 * 1024, 123));
    let server = Server::start(
        {
            let data = data.clone();
            Arc::new(move |_, range| range_response(&data, range))
        },
        true,
    )
    .await;
    let torrent = tests::with_seed(
        single_file_torrent("test.bin", &data, 32768),
        "url-list",
        &format!("{}/test.bin", server.base),
    );
    let hash = meta_of(&torrent).info_hash;
    let folder = TempFolder::new();
    let state = folder.path().join("state");
    let output = folder.path().join("output");
    let first = session(state.clone()).await;
    first
        .add_torrent_file(
            "download".into(),
            &torrent,
            AddOptions {
                output: output.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    wait_for(&first, |s| s.progress_bytes > 0 && !s.finished).await;
    first.stop_all().await;
    let progress = first.stats().torrents["download"].progress_bytes;
    let snapshot = std::fs::read(output.join("test.bin")).unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(std::fs::read(output.join("test.bin")).unwrap(), snapshot);
    assert_eq!(first.stats().torrents["download"].progress_bytes, progress);
    first.shutdown().await;
    // Simulate an incomplete resume record after a crash. Rehash disk pieces and retain seeds.
    std::fs::write(
        state.join(format!("{}.resume.json", hash.as_string())),
        b"{unfinished",
    )
    .unwrap();
    let second = session(state).await;
    second
        .add_magnet(
            "download".into(),
            &format!("magnet:?xt=urn:btih:{}", hash.as_string()),
            AddOptions {
                output: output.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    wait_for(&second, |s| s.finished).await;
    assert_eq!(second.stats().managed, 1);
    assert_eq!(std::fs::read(output.join("test.bin")).unwrap(), *data);
    let requests = server.requests.lock().clone();
    assert!(
        requests.len() < 10,
        "Requests must span contiguous pieces, not one request per chunk: {}",
        requests.len()
    );
    assert!(
        requests
            .iter()
            .any(|(_, range)| range.starts_with("bytes=4194304-"))
    );
    second.shutdown().await;
    server.stop().await;
}

#[tokio::test]
async fn multifile_seed_crosses_file_boundaries_using_original_urls() {
    let a = sample_bytes(21000, 1);
    let b = sample_bytes(33000, 2);
    let c = sample_bytes(41000, 3);
    let routes = BTreeMap::from([
        ("/root/a%20b/one.bin".to_owned(), a.clone()),
        ("/root/a%20b/two%3F.bin".to_owned(), b.clone()),
        ("/root/a%20b/three.bin".to_owned(), c.clone()),
    ]);
    let server = Server::start(
        Arc::new(move |path, range| range_response(&routes[path], range)),
        false,
    )
    .await;
    let torrent = tests::with_seed(
        multi_file_torrent(
            "a b",
            &[
                (&["one.bin"], &a, false),
                (&["two?.bin"], &b, false),
                (&["three.bin"], &c, false),
            ],
            16384,
        ),
        "url-list",
        &format!("{}/root/", server.base),
    );
    let meta = meta_of(&torrent);
    let folder = TempFolder::new();
    let session = session(folder.path().join("state")).await;
    session
        .add_torrent_file(
            "download".into(),
            &torrent,
            AddOptions {
                output: folder.path().join("output"),
                only: Some(vec![0, 2]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    wait_for(&session, |s| s.finished).await;
    assert_eq!(
        std::fs::read(folder.path().join("output").join(&meta.files[0].path)).unwrap(),
        a
    );
    assert_eq!(
        std::fs::read(folder.path().join("output").join(&meta.files[2].path)).unwrap(),
        c
    );
    session.shutdown().await;
    server.stop().await;
}

#[tokio::test]
async fn http_seed_serves_binary_hash_and_piece_relative_ranges() {
    let data = Arc::new(sample_bytes(100000, 511));
    let torrent = single_file_torrent("test.bin", &data, 32768);
    let hash = meta_of(&torrent).info_hash;
    let handler = {
        let data = data.clone();
        Arc::new(move |target: &str, _: &str| {
            let url = Url::parse(&format!("http://fixture{target}")).unwrap();
            let query = url.query_pairs().collect::<BTreeMap<_, _>>();
            let piece: usize = query["piece"].parse().unwrap();
            assert!(url.query().unwrap().contains(&format!(
                    "info_hash={}",
                    hash.0
                        .iter()
                        .map(|b| format!("%{b:02X}"))
                        .collect::<String>()
                )));
            let (a, b) = query["ranges"].split_once('-').unwrap();
            let a: usize = a.parse().unwrap();
            let b: usize = b.parse().unwrap();
            response(
                "200 OK",
                "",
                data[piece * 32768 + a..=piece * 32768 + b].to_vec(),
            )
        })
    };
    let server = Server::start(handler, false).await;
    let torrent = tests::with_seed(
        torrent,
        "httpseeds",
        &format!("{}/seed?token=fixture", server.base),
    );
    let folder = TempFolder::new();
    let session = session(folder.path().join("state")).await;
    session
        .add_torrent_file(
            "download".into(),
            &torrent,
            AddOptions {
                output: folder.path().join("output"),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    wait_for(&session, |s| s.finished).await;
    assert_eq!(
        std::fs::read(folder.path().join("output/test.bin")).unwrap(),
        *data
    );
    session.shutdown().await;
    server.stop().await;
}

#[tokio::test]
async fn rejects_redirect_wrong_range_oversized_body_and_honors_retry() {
    for variant in 0..4 {
        let server=Server::start(Arc::new(move|_,_|match variant {
            0=>response("302 Found","Location: http://127.0.0.1:1/private\r\n",vec![]),
            1=>response("206 Partial Content","Content-Range: bytes 1-4/5\r\n",vec![0;4]),
            2=>("HTTP/1.1 206 Partial Content\r\nTransfer-Encoding: chunked\r\nContent-Range: bytes 0-3/5\r\nConnection: close\r\n\r\n".into(),b"5\r\nabcde\r\n0\r\n\r\n".to_vec()),
            _=>response("503 Unavailable","",b"120".to_vec()),
        }),false).await;
        let error = Http::new(true)
            .unwrap()
            .range(Url::parse(&server.base).unwrap(), 0, 4, 5, false, None)
            .await
            .unwrap_err();
        if variant == 3 {
            assert_eq!(
                error.downcast_ref::<RetryAfter>().unwrap().0,
                Duration::from_secs(120)
            );
        }
        assert_eq!(server.requests.lock().len(), 1);
        server.stop().await;
    }
}

#[tokio::test]
async fn corrupted_seed_never_marks_progress_and_is_not_retried() {
    let data = sample_bytes(65536, 76);
    let server = Server::start(
        Arc::new(move |_, range| range_response(&vec![0; 65536], range)),
        false,
    )
    .await;
    let torrent = tests::with_seed(
        single_file_torrent("test.bin", &data, 16384),
        "url-list",
        &server.base,
    );
    let folder = TempFolder::new();
    let session = session(folder.path().join("state")).await;
    session
        .add_torrent_file(
            "download".into(),
            &torrent,
            AddOptions {
                output: folder.path().join("output"),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    timeout(Duration::from_secs(5), async {
        while server.requests.lock().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(session.stats().torrents["download"].progress_bytes, 0);
    assert_eq!(server.requests.lock().len(), 1);
    session.shutdown().await;
    server.stop().await;
}
