//! Opt-in, byte-verified loopback comparisons; timings are observations, not assertions.
use download_engine::{
    bandwidth::Limiter,
    engine, hls,
    model::{Job, Status},
    paths::AppPaths,
    queue::MAIN_QUEUE,
    quota,
    store::Store,
};
use sha2::{Digest, Sha256};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

const PARTS: usize = 16;
const VIDEO_PART: usize = 188 * 4096;
const FILE_BYTES: usize = 32 * 1024 * 1024;

struct Fixture {
    url: String,
    file: Arc<Vec<u8>>,
    video: Arc<Vec<u8>>,
    peak: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl Fixture {
    async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let file = Arc::new(
            (0..FILE_BYTES)
                .map(|i| ((i ^ (i >> 9)) % 251) as u8)
                .collect::<Vec<_>>(),
        );
        let mut video = vec![19; VIDEO_PART];
        for packet in video.chunks_mut(188) {
            packet[0] = 0x47;
        }
        let video = Arc::new(video);
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (f, v, p) = (file.clone(), video.clone(), peak.clone());
        let task = tokio::spawn(async move {
            let mut peers = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((socket, _)) = accepted else { break };
                        let (f, v, a, p) = (f.clone(), v.clone(), active.clone(), p.clone());
                        peers.spawn(async move { let _ = serve(socket, f, v, a, p).await; });
                    }
                    _ = peers.join_next(), if !peers.is_empty() => {}
                }
            }
        });
        Self {
            url,
            file,
            video,
            peak,
            task,
        }
    }
}
async fn serve(
    mut socket: TcpStream,
    file: Arc<Vec<u8>>,
    video: Arc<Vec<u8>>,
    active: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
) -> std::io::Result<()> {
    let mut request = Vec::new();
    let mut buffer = [0; 4096];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") && request.len() < 32768 {
        let n = socket.read(&mut buffer).await?;
        if n == 0 {
            return Ok(());
        }
        request.extend_from_slice(&buffer[..n]);
    }
    let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
    let path = request
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    if path == "/video.m3u8" {
        let body = format!(
            "#EXTM3U\n#EXT-X-TARGETDURATION:6\n{}#EXT-X-ENDLIST\n",
            (0..PARTS)
                .map(|i| format!("#EXTINF:6,\npart-{i}.ts\n"))
                .collect::<String>()
        );
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            )
            .await?;
        socket.write_all(body.as_bytes()).await?;
        return Ok(());
    }
    let media = path.starts_with("/part-");
    let payload = if media { &video } else { &file };
    let range = if path == "/range.bin" {
        request.lines().find_map(|line| {
            let value = line.strip_prefix("range: bytes=")?;
            let (a, b) = value.split_once('-')?;
            Some((a.parse::<usize>().unwrap(), b.parse::<usize>().unwrap()))
        })
    } else {
        None
    };
    let (start, end) = range.unwrap_or((0, payload.len() - 1));
    let probe = range == Some((0, 0));
    let _active = if !probe {
        peak.fetch_max(active.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
        Some(Active(active))
    } else {
        None
    };
    let extra = if range.is_some() {
        format!("Content-Range: bytes {start}-{end}/{}\r\n", payload.len())
    } else {
        String::new()
    };
    socket.write_all(format!("HTTP/1.1 {}\r\nContent-Length: {}\r\nETag: \"stable\"\r\n{extra}Connection: close\r\n\r\n",
        if range.is_some() { "206 Partial Content" } else { "200 OK" }, end - start + 1).as_bytes()).await?;
    for chunk in payload[start..=end].chunks(if media { 64 * 1024 } else { 16 * 1024 }) {
        if media {
            tokio::time::sleep(Duration::from_millis(3)).await;
        }
        socket.write_all(chunk).await?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "explicit release-mode throughput benchmark"]
async fn transfer_throughput() {
    for (scenario, path, limited) in [
        ("video", "/video.m3u8", false),
        ("video_quota", "/video.m3u8", true),
        ("file", "/file.bin", false),
        ("ranges", "/range.bin", false),
        ("ranges_quota", "/range.bin", true),
    ] {
        for sample in 0..3 {
            let fixture = Fixture::new().await;
            let temp = tempfile::tempdir().unwrap();
            let store = Store::open(AppPaths {
                data: temp.path().join("data"),
                downloads: temp.path().join("downloads"),
            })
            .unwrap();
            if limited {
                store
                    .set_quota_limits_at(
                        quota::Limits {
                            hourly: Some(1024 * 1024 * 1024),
                            daily: None,
                        },
                        chrono::Utc::now().timestamp(),
                    )
                    .unwrap();
            }
            let (mut job, bytes, expected) = if scenario.starts_with("video") {
                let playlist =
                    hls::inspect(&format!("{}{path}", fixture.url), &CancellationToken::new())
                        .await
                        .unwrap()
                        .playlist
                        .unwrap();
                let job = store
                    .enqueue_hls(
                        &playlist,
                        "video.ts",
                        None,
                        MAIN_QUEUE,
                        &uuid::Uuid::new_v4().to_string(),
                    )
                    .unwrap();
                let mut hash = Sha256::new();
                for _ in 0..PARTS {
                    hash.update(&*fixture.video);
                }
                (job, VIDEO_PART * PARTS, format!("{:x}", hash.finalize()))
            } else {
                let job = Job::new(
                    &format!("{}{path}", fixture.url),
                    Some("file.bin"),
                    store.paths.downloads.clone(),
                    uuid::Uuid::new_v4().to_string(),
                )
                .unwrap();
                let job = store.enqueue(&job).unwrap();
                (
                    job,
                    FILE_BYTES,
                    format!("{:x}", Sha256::digest(&*fixture.file)),
                )
            };
            job.status = Status::Downloading;
            let id = job.id.clone();
            let started = Instant::now();
            engine::run(
                store.clone(),
                job,
                CancellationToken::new(),
                Limiter::default(),
            )
            .await
            .unwrap();
            let elapsed = started.elapsed();
            let finished = store.get(&id).unwrap().unwrap();
            assert_eq!(finished.status, Status::Complete, "{:?}", finished.error);
            assert_eq!(finished.downloaded, bytes as u64);
            assert_eq!(
                finished.integrity.actual.as_deref(),
                Some(expected.as_str())
            );
            let output = std::fs::read(finished.saved_path.unwrap()).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(&output)), expected);
            if limited {
                assert_eq!(store.quota_status().unwrap().hourly_used, bytes as u64);
            }
            println!(
                "BENCH {}",
                serde_json::json!({ "scenario": scenario, "sample": sample,
                "bytes": bytes, "milliseconds": elapsed.as_secs_f64() * 1000.0,
                "mib_per_second": bytes as f64 / 1048576.0 / elapsed.as_secs_f64(),
                "peak_connections": fixture.peak.load(Ordering::SeqCst), "sha256_verified": true })
            );
        }
    }
}
