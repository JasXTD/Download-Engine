//! Deterministic loopback coverage for connection handoff and persisted parts.
use download_engine::{
    bandwidth::Limiter,
    engine,
    model::{Job, Plan, Segment, Status},
    paths::AppPaths,
    store::Store,
};
use sha2::{Digest, Sha256};
use std::{
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Semaphore,
    task::{JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;

const MIB: usize = 1024 * 1024;
const SIZE: usize = 32 * MIB;
const CHUNK: usize = 128 * 1024;
#[derive(Clone, Debug)]
struct Request {
    start: usize,
    end: usize,
    validator: bool,
}
struct Fixture {
    url: String,
    data: Arc<Vec<u8>>,
    fault: Arc<AtomicU8>,
    gate: Arc<Semaphore>,
    requests: Arc<Mutex<Vec<Request>>>,
    peak: Arc<AtomicUsize>,
    task: JoinHandle<()>,
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
    async fn new(size: usize, skew: bool, fault: u8) -> Self {
        Self::with_donor_gate(size, skew, fault, true).await
    }
    async fn with_donor_gate(size: usize, skew: bool, fault: u8, hold_donor: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/data.bin", listener.local_addr().unwrap());
        let data = Arc::new(
            (0..size)
                .map(|i| ((i ^ (i >> 11)) % 251) as u8)
                .collect::<Vec<_>>(),
        );
        let fault = Arc::new(AtomicU8::new(fault));
        let gate = Arc::new(Semaphore::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (d, f, g, r, p) = (
            data.clone(),
            fault.clone(),
            gate.clone(),
            requests.clone(),
            peak.clone(),
        );
        let task = tokio::spawn(async move {
            let mut tasks = JoinSet::new();
            loop {
                tokio::select! {
                    stream = listener.accept() => {
                        let Ok((stream, _)) = stream else { break };
                        let (d, f, g, r, a, p) = (
                            d.clone(), f.clone(), g.clone(), r.clone(), active.clone(), p.clone(),
                        );
                        tasks.spawn(async move { serve(stream, d, skew, hold_donor, f, g, r, a, p).await });
                    }
                    _ = tasks.join_next(), if !tasks.is_empty() => {}
                }
            }
        });
        Self {
            url,
            data,
            fault,
            gate,
            requests,
            peak,
            task,
        }
    }
    fn release(&self) {
        self.fault.store(0, Ordering::SeqCst);
        self.gate.add_permits(1);
    }
}
#[allow(clippy::too_many_arguments)]
async fn serve(
    mut stream: TcpStream,
    data: Arc<Vec<u8>>,
    skew: bool,
    hold_donor: bool,
    fault: Arc<AtomicU8>,
    gate: Arc<Semaphore>,
    requests: Arc<Mutex<Vec<Request>>>,
    active: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
) -> io::Result<()> {
    let mut raw = Vec::new();
    let mut buf = [0; 4096];
    while !raw.ends_with(b"\r\n\r\n") && raw.len() < 32768 {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            return Ok(());
        }
        raw.extend_from_slice(&buf[..n]);
    }
    let raw = String::from_utf8(raw).unwrap().to_ascii_lowercase();
    let (start, end) = raw
        .lines()
        .find_map(|line| {
            let range = line.strip_prefix("range: bytes=")?;
            let (a, b) = range.split_once('-')?;
            Some((a.parse::<usize>().unwrap(), b.parse::<usize>().unwrap()))
        })
        .expect("All requests must be bounded ranges");
    assert!(end < data.len() && start <= end);
    let probe = start == 0 && end == 0;
    let step = data.len() / 4;
    let initial = start.is_multiple_of(step) && end == start + step - 1;
    requests.lock().unwrap().push(Request {
        start,
        end,
        validator: raw.contains("if-range: \"stable\""),
    });
    let mode = if initial || probe {
        0
    } else {
        fault.load(Ordering::SeqCst)
    };
    let _active = if !probe {
        peak.fetch_max(active.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
        Some(Active(active))
    } else {
        None
    };
    let (mut read, mut write) = stream.into_split();
    let mut eof = [0];
    // Watch peer closure while a slow or gated body is outstanding.
    tokio::select! {
        _ = read.read(&mut eof) => Ok(()),
        result = async {
            if mode == 2 {
                write.write_all(b"HTTP/1.1 503 Unavailable\r\nRetry-After: 1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await?;
                return Ok(());
            }
            let etag = if mode == 3 { "changed" } else { "stable" };
            let status = if mode == 4 { "200 OK" } else { "206 Partial Content" };
            let reply = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{end}/{}\r\nETag: \"{etag}\"\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                end-start+1, data.len()
            );
            write.write_all(reply.as_bytes()).await?;
            for (n, bytes) in data[start..=end].chunks(CHUNK).enumerate() {
                if !probe {
                    // Split regressions need a remaining donor even when other
                    // test processes contend for disk/CPU. Cancellation closes
                    // this socket; the benchmark keeps its original timed body.
                    if n > 0 && (mode == 1 || (hold_donor && skew && initial && start == 0)) {
                        let _permit = gate.acquire().await.unwrap();
                    }
                    if mode == 5 && n > 0 { return Ok(()) }
                    let delay = if skew && initial && start == 0 { 50 } else { 3 };
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                }
                if mode == 6 {
                    let changed: Vec<_> = bytes.iter().map(|b| b ^ 1).collect();
                    write.write_all(&changed).await?;
                } else {
                    write.write_all(bytes).await?;
                }
            }
            Ok(())
        } => result,
    }
}
fn setup(fixture: &Fixture) -> (TempDir, Store, Job) {
    let temp = TempDir::new().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    let mut job = Job::new(
        &fixture.url,
        Some("data.bin"),
        store.paths.downloads.clone(),
        uuid::Uuid::new_v4().to_string(),
    )
    .unwrap();
    job.status = Status::Downloading;
    job.integrity.expected = Some(format!("{:x}", Sha256::digest(&*fixture.data)));
    store.enqueue(&job).unwrap();
    (temp, store, job)
}
async fn run(store: &Store, job: Job) -> Job {
    let id = job.id.clone();
    let limiter = Limiter::new(&store.bandwidth_limits().unwrap()).unwrap();
    engine::run(store.clone(), job, CancellationToken::new(), limiter)
        .await
        .unwrap();
    store.get(&id).unwrap().unwrap()
}
fn complete(fixture: &Fixture, job: &Job) {
    complete_with_limit(fixture, job, 4);
}
fn complete_with_limit(fixture: &Fixture, job: &Job, limit: usize) {
    assert_eq!(job.status, Status::Complete, "{:?}", job.error);
    assert_eq!(
        std::fs::read(job.saved_path.as_ref().unwrap()).unwrap(),
        *fixture.data
    );
    assert_eq!(job.integrity.actual, job.integrity.expected);
    assert!(
        fixture.peak.load(Ordering::SeqCst) <= limit,
        "too many simultaneous body requests"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connection_settings_control_real_worker_concurrency_and_preserve_bytes() {
    use download_engine::connections::{Settings, Site};
    for (default_connections, site_limit, expected) in [(1, None, 1), (8, None, 8), (8, Some(2), 2)]
    {
        let fixture = Fixture::with_donor_gate(SIZE, false, 0, false).await;
        let (_temp, store, job) = setup(&fixture);
        let settings = Settings {
            default_connections,
            sites: site_limit
                .map(|connections| Site {
                    host: "127.0.0.1".into(),
                    subdomains: false,
                    connections,
                })
                .into_iter()
                .collect(),
        };
        store
            .save_connection_settings(&Settings::default(), settings)
            .unwrap();
        let finished = run(&store, job).await;
        complete_with_limit(&fixture, &finished, expected);
        assert_eq!(
            fixture.peak.load(Ordering::SeqCst),
            expected,
            "the selected limit must reach actual worker scheduling"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lower_connection_limit_resumes_an_existing_sixteen_part_plan() {
    use download_engine::connections::Settings;
    let fixture = Fixture::with_donor_gate(SIZE, false, 0, false).await;
    let (_temp, store, mut job) = setup(&fixture);
    let root = store.paths.parts(&job.id).unwrap();
    std::fs::create_dir(&root).unwrap();
    let step = SIZE / 16;
    let segments = (0..16)
        .map(|i| {
            let start = i * step;
            std::fs::write(
                root.join(format!("segment-{i}.part")),
                &fixture.data[start..start + step / 2],
            )
            .unwrap();
            Segment {
                start: start as u64,
                end: (start + step - 1) as u64,
            }
        })
        .collect();
    job.plan = Some(Plan {
        size: SIZE as u64,
        etag: "\"stable\"".into(),
        final_url: fixture.url.clone(),
        segments,
    });
    job.downloaded = (SIZE / 2) as u64;
    job.total = Some(SIZE as u64);
    store.save(&job).unwrap();
    store
        .save_connection_settings(
            &Settings::default(),
            Settings {
                default_connections: 1,
                sites: vec![],
            },
        )
        .unwrap();
    let finished = run(&store, job).await;
    complete_with_limit(&fixture, &finished, 1);
    let requested: usize = fixture
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.end > 0)
        .map(|r| r.end - r.start + 1)
        .sum();
    assert_eq!(
        requested,
        SIZE / 2,
        "existing bytes must not be downloaded again"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn redirected_download_uses_the_stricter_source_and_destination_rules() {
    use download_engine::connections::{Settings, Site};
    let fixture = Fixture::with_donor_gate(SIZE, false, 0, false).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let source = format!(
        "http://localhost:{}/redirect",
        listener.local_addr().unwrap().port()
    );
    let destination = fixture.url.clone();
    let redirect = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 4096];
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") && request.len() < 32768 {
                let count = stream.read(&mut bytes).await.unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&bytes[..count]);
            }
            stream.write_all(format!("HTTP/1.1 302 Found\r\nLocation: {destination}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        }
    });
    let (_temp, store, mut job) = setup(&fixture);
    job.url = source;
    store.save(&job).unwrap();
    store
        .save_connection_settings(
            &Settings::default(),
            Settings {
                default_connections: 8,
                sites: vec![
                    Site {
                        host: "localhost".into(),
                        subdomains: false,
                        connections: 1,
                    },
                    Site {
                        host: "127.0.0.1".into(),
                        subdomains: false,
                        connections: 8,
                    },
                ],
            },
        )
        .unwrap();
    let finished = run(&store, job).await;
    redirect.abort();
    complete_with_limit(&fixture, &finished, 1);
    assert_eq!(fixture.peak.load(Ordering::SeqCst), 1);
}
fn saved_parts(store: &Store, job: &Job, data: &[u8]) -> Vec<usize> {
    let plan = job.plan.as_ref().unwrap();
    let mut sum = 0;
    let mut lengths = Vec::new();
    for (i, part) in plan.segments.iter().enumerate() {
        let path = store
            .paths
            .parts(&job.id)
            .unwrap()
            .join(format!("segment-{i}.part"));
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => panic!("{error}"),
        };
        assert!(bytes.len() as u64 <= part.end - part.start + 1);
        assert_eq!(
            bytes,
            data[part.start as usize..part.start as usize + bytes.len()]
        );
        sum += bytes.len();
        lengths.push(bytes.len());
    }
    assert_eq!(sum as u64, job.downloaded);
    lengths
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn adaptive_reassigns_unwritten_tail_and_verifies_exact_bytes() {
    let fixture = Fixture::new(SIZE, true, 0).await;
    let (_temp, store, job) = setup(&fixture);
    let finished = run(&store, job).await;
    complete(&fixture, &finished);
    assert!(finished.plan.as_ref().unwrap().segments.len() > 4);
    assert!(
        fixture
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.end > 0)
            .all(|r| r.validator)
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn adaptive_pause_reopens_database_and_resumes_only_unsaved_bytes() {
    let fixture = Fixture::new(SIZE, true, 1).await;
    let (_temp, store, job) = setup(&fixture);
    let id = job.id.clone();
    let token = CancellationToken::new();
    let worker = tokio::spawn(engine::run(
        store.clone(),
        job,
        token.clone(),
        Limiter::new(&store.bandwidth_limits().unwrap()).unwrap(),
    ));
    let observed = tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            let (s, key) = (store.clone(), id.clone());
            let saved = tokio::task::spawn_blocking(move || s.get(&key).unwrap().unwrap())
                .await
                .unwrap();
            if saved.plan.as_ref().is_some_and(|p| p.segments.len() > 4) {
                break;
            }
            assert!(
                !worker.is_finished(),
                "worker stopped before a split: {saved:?}"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    token.cancel();
    worker.await.unwrap().unwrap();
    observed.expect("split did not occur");
    let paused = store.get(&id).unwrap().unwrap();
    assert_eq!(paused.status, Status::Paused);
    assert!(paused.downloaded > 0 && paused.downloaded < SIZE as u64);
    let lengths = saved_parts(&store, &paused, &fixture.data);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        saved_parts(&store, &paused, &fixture.data),
        lengths,
        "writes outlived pause"
    );
    let layout = serde_json::to_value(paused.plan.as_ref().unwrap()).unwrap();
    assert_eq!(layout["segments"]["version"], 2);
    // Old readers expected a Vec: they must reject, never silently concatenate
    // the adaptive physical file order.
    assert!(serde_json::from_value::<Vec<Segment>>(layout["segments"].clone()).is_err());
    let reopened = Store::open(store.paths.clone()).unwrap();
    let persisted = reopened.get(&id).unwrap().unwrap();
    fixture.requests.lock().unwrap().clear();
    fixture.release();
    let finished = run(&reopened, persisted).await;
    complete(&fixture, &finished);
    for request in fixture
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.end > 0)
    {
        let (index, segment) = paused
            .plan
            .as_ref()
            .unwrap()
            .segments
            .iter()
            .enumerate()
            .find(|(_, p)| request.start as u64 >= p.start && request.start as u64 <= p.end)
            .unwrap();
        assert!(
            request.start as u64 >= segment.start + lengths[index] as u64,
            "saved bytes requested again"
        );
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_and_many_part_resume_respect_connection_cap_and_byte_order() {
    let fixture = Fixture::new(4 * MIB, false, 0).await;
    let (_temp, store, mut job) = setup(&fixture);
    let step = fixture.data.len() / 16;
    let segments: Vec<_> = (0..16)
        .rev()
        .map(|i| Segment {
            start: (i * step) as u64,
            end: ((i + 1) * step - 1) as u64,
        })
        .collect();
    let root = store.paths.parts(&job.id).unwrap();
    std::fs::create_dir(&root).unwrap();
    for (i, part) in segments.iter().enumerate() {
        // A crash after committing a split can leave its new part absent.
        if i % 4 == 1 {
            continue;
        }
        let len = if i % 3 == 0 { step } else { step / 2 };
        let start = part.start as usize;
        std::fs::write(
            root.join(format!("segment-{i}.part")),
            &fixture.data[start..start + len],
        )
        .unwrap();
        job.downloaded += len as u64;
    }
    job.plan = Some(Plan {
        size: fixture.data.len() as u64,
        etag: "\"stable\"".into(),
        final_url: fixture.url.clone(),
        segments,
    });
    job.total = Some(fixture.data.len() as u64);
    let old_bytes = job.downloaded;
    job.downloaded = 0; // Simulate a stale progress checkpoint.
    store.save(&job).unwrap();
    let finished = run(&store, store.get(&job.id).unwrap().unwrap()).await;
    complete(&fixture, &finished);
    let requested: usize = fixture
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.end > 0)
        .map(|r| r.end - r.start + 1)
        .sum();
    assert_eq!(requested as u64, fixture.data.len() as u64 - old_bytes);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn adaptive_failures_keep_safe_parts_and_retry_transient_responses() {
    for mode in [2, 3, 4, 5, 6] {
        let fixture = Fixture::new(SIZE, true, mode).await;
        let (_temp, store, job) = setup(&fixture);
        let failed = run(&store, job).await;
        assert_ne!(failed.status, Status::Complete, "mode {mode}");
        assert!(failed.saved_path.is_none());
        assert!(!store.paths.downloads.join("data.bin").exists());
        assert!(failed.plan.as_ref().unwrap().segments.len() > 4);
        assert!(fixture.peak.load(Ordering::SeqCst) <= 4);
        if mode != 6 {
            saved_parts(&store, &failed, &fixture.data);
        }
        if mode == 2 || mode == 5 {
            assert_eq!(failed.retry.attempts, 1, "{failed:?}");
            assert!(failed.retry.next_at.is_some());
            fixture.release();
            complete(&fixture, &run(&store, failed).await);
        } else {
            assert_eq!(failed.status, Status::Failed);
            assert!(failed.retry.next_at.is_none());
            if mode == 6 {
                assert!(
                    failed.error.as_ref().unwrap().contains("SHA-256"),
                    "{failed:?}"
                );
            }
        }
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_split_commit_preserves_original_layout_and_files() {
    let fixture = Fixture::new(SIZE, true, 0).await;
    let (_temp, store, job) = setup(&fixture);
    let db = rusqlite::Connection::open(store.paths.data.join("queue.sqlite3")).unwrap();
    db.execute_batch("CREATE TRIGGER reject_split BEFORE UPDATE ON jobs WHEN json_type(NEW.data,'$.plan.segments')='object' BEGIN SELECT RAISE(FAIL,'injected split commit failure'); END;").unwrap();
    let failed = run(&store, job).await;
    assert_eq!(failed.status, Status::Failed);
    assert!(
        failed
            .error
            .as_ref()
            .unwrap()
            .contains("injected split commit failure")
    );
    assert_eq!(failed.plan.as_ref().unwrap().segments.len(), 4);
    saved_parts(&store, &failed, &fixture.data);
    assert!(
        !store
            .paths
            .parts(&failed.id)
            .unwrap()
            .join("segment-4.part")
            .exists()
    );
    db.execute_batch("DROP TRIGGER reject_split").unwrap();
    complete(&fixture, &run(&store, failed).await);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn adaptive_split_refuses_unaccounted_part_file() {
    let fixture = Fixture::new(SIZE, true, 0).await;
    let (_temp, store, job) = setup(&fixture);
    let root = store.paths.parts(&job.id).unwrap();
    std::fs::create_dir(&root).unwrap();
    let unexpected = root.join("segment-4.part");
    std::fs::write(&unexpected, b"preserve me").unwrap();
    let failed = run(&store, job).await;
    assert_eq!(failed.status, Status::Failed);
    assert_eq!(failed.plan.as_ref().unwrap().segments.len(), 4);
    assert_eq!(std::fs::read(unexpected).unwrap(), b"preserve me");
    saved_parts(&store, &failed, &fixture.data);
}

// Run unchanged against the checkpoint and the new source. Timing is a report,
// never a flaky pass/fail threshold; every sample still checks exact bytes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "explicit loopback throughput comparison"]
async fn uniform_and_uneven_benchmark() {
    for skew in [false, true] {
        for sample in 0..3 {
            let fixture = Fixture::with_donor_gate(SIZE, skew, 0, false).await;
            let (_temp, store, job) = setup(&fixture);
            let start = Instant::now();
            let finished = run(&store, job).await;
            let ms = start.elapsed().as_millis();
            complete(&fixture, &finished);
            println!(
                "BENCH {}",
                serde_json::json!({
                    "scenario": if skew { "uneven" } else { "uniform" },
                    "sample": sample, "bytes": SIZE, "milliseconds": ms,
                    "parts": finished.plan.as_ref().unwrap().segments.len(),
                    "requests": fixture.requests.lock().unwrap().len(),
                    "peak_connections": fixture.peak.load(Ordering::SeqCst),
                })
            );
        }
    }
}
