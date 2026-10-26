use download_engine::{
    engine,
    model::{Job, Status, safe_filename, validate_url},
    native,
    paths::AppPaths,
    scheduler::{Command, Scheduler},
    store::Store,
};
use std::{
    io::{Cursor, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

struct Fixture {
    url: String,
    data: Arc<Vec<u8>>,
    mode: Arc<AtomicU8>,
    requests: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
    disposition: Arc<Mutex<Vec<String>>>,
    body_gate: Option<Arc<tokio::sync::Semaphore>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    async fn new(size: usize, mode: u8, slow: bool) -> Self {
        Self::with_body_gate(size, mode, slow, None).await
    }
    async fn with_body_gate(
        size: usize,
        mode: u8,
        slow: bool,
        body_gate: Option<Arc<tokio::sync::Semaphore>>,
    ) -> Self {
        Self::with_gate_after(size, mode, slow, body_gate.map(|gate| (0, gate))).await
    }
    async fn pausable_range(size: usize, mode: u8) -> Self {
        Self::with_gate_after(
            size,
            mode,
            false,
            Some((128 * 1024, Arc::new(tokio::sync::Semaphore::new(0)))),
        )
        .await
    }
    fn release_body(&self) {
        if let Some(gate) = &self.body_gate {
            gate.add_permits(1);
        }
    }
    async fn with_gate_after(
        size: usize,
        mode: u8,
        slow: bool,
        body_gate: Option<(usize, Arc<tokio::sync::Semaphore>)>,
    ) -> Self {
        let release = body_gate.as_ref().map(|(_, gate)| gate.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/fixture.bin", listener.local_addr().unwrap());
        let data = Arc::new((0..size).map(|i| (i % 251) as u8).collect::<Vec<_>>());
        let mode = Arc::new(AtomicU8::new(mode));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let disposition = Arc::new(Mutex::new(Vec::<String>::new()));
        let headers = disposition.clone();
        let (d, m, r) = (data.clone(), mode.clone(), requests.clone());
        let task = tokio::spawn(async move {
            let mut handlers = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    result=listener.accept()=>{
                        let Ok((stream,_))=result else {break;};
                        let (d,m,r)=(d.clone(),m.clone(),r.clone());
                        let body_gate = body_gate.clone();
                        let headers = headers.clone();
                        handlers.spawn(async move {let _=serve(stream,d,m,r,slow,body_gate,headers).await;});
                    },
                    _=handlers.join_next(),if !handlers.is_empty()=>{}
                }
            }
        });
        Self {
            url,
            data,
            mode,
            requests,
            task,
            disposition,
            body_gate: release,
        }
    }
}
async fn serve(
    mut stream: TcpStream,
    data: Arc<Vec<u8>>,
    mode: Arc<AtomicU8>,
    requests: Arc<Mutex<Vec<String>>>,
    slow: bool,
    body_gate: Option<(usize, Arc<tokio::sync::Semaphore>)>,
    disposition: Arc<Mutex<Vec<String>>>,
) -> std::io::Result<()> {
    let mut raw = Vec::new();
    let mut byte = [0];
    while raw.len() < 32768 {
        if stream.read(&mut byte).await? == 0 {
            return Ok(());
        }
        raw.push(byte[0]);
        if raw.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let request = String::from_utf8(raw).unwrap();
    requests.lock().unwrap().push(request.clone());
    let range = request.lines().find_map(|line| {
        line.to_ascii_lowercase()
            .strip_prefix("range: bytes=")
            .map(str::to_owned)
    });
    let mode = mode.load(Ordering::SeqCst);
    if matches!(mode, 16 | 17) {
        let cookie = request.lines().find_map(|line| {
            line.split_once(':')
                .filter(|(name, _)| name.eq_ignore_ascii_case("cookie"))
                .map(|(_, value)| value.trim())
        });
        if !cookie.is_some_and(|value| {
            value
                .split("; ")
                .any(|c| c == "session=SYNTHETIC-BROWSER-LOGIN")
        }) {
            stream
                .write_all(
                    b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await?;
            return Ok(());
        }
    }
    if mode == 17 && range.as_deref() != Some("0-0") {
        stream.write_all(b"HTTP/1.1 503 Unavailable\r\nContent-Type: text/html\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await?;
        return Ok(());
    }
    if matches!(mode, 11..=14) {
        let expected = if mode == 11 {
            "Basic dGVzdC11c2VyOnRlc3QtcGFzcw=="
        } else {
            "Bearer test-token_123"
        };
        let actual = request.lines().find_map(|line| {
            line.split_once(':')
                .filter(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                .map(|(_, value)| value.trim())
        });
        if actual != Some(expected) {
            stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"test\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await?;
            return Ok(());
        }
    }
    if let Some(target) = request
        .split_whitespace()
        .nth(1)
        .and_then(|path| path.strip_prefix("/cross-origin/"))
    {
        let port: u16 = target.parse().unwrap();
        let reply = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}/fixture.bin\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(reply.as_bytes()).await?;
        return Ok(());
    }
    if mode == 15 && range.as_deref() != Some("0-0") {
        stream.write_all(b"HTTP/1.1 503 Unavailable\r\nRetry-After: 7\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await?;
        return Ok(());
    }
    if mode == 7 {
        let extra = disposition
            .lock()
            .unwrap()
            .iter()
            .map(|value| format!("Content-Disposition: {value}\r\n"))
            .collect::<String>();
        stream.write_all(format!("HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n{extra}\r\n").as_bytes()).await?;
        return Ok(());
    }
    if request.starts_with("GET /redirect ") {
        stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: /fixture.bin\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await?;
        return Ok(());
    }
    if data.is_empty() && range.is_some() && mode == 16 {
        stream.write_all(b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Type: text/html\r\nContent-Range: bytes */0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await?;
        return Ok(());
    }
    if data.is_empty() && range.is_some() {
        stream.write_all(b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await?;
        return Ok(());
    }
    let (start, end, partial) = if !matches!(mode, 1 | 13)
        && let Some(range) = range
    {
        let (a, b) = range.split_once('-').unwrap();
        (
            a.parse::<usize>().unwrap(),
            b.parse::<usize>().unwrap(),
            true,
        )
    } else {
        (0, data.len().saturating_sub(1), false)
    };
    let count = if data.is_empty() { 0 } else { end - start + 1 };
    let mut headers = format!(
        "HTTP/1.1 {} OK\r\nContent-Length: {count}\r\nConnection: close\r\n",
        if partial { 206 } else { 200 }
    );
    if partial {
        headers.push_str(&format!(
            "Content-Range: bytes {}-{end}/{}\r\n",
            if mode == 3 && count > 1 {
                start + 1
            } else {
                start
            },
            data.len()
        ));
    }
    if !matches!(mode, 5 | 14) {
        headers.push_str(&format!(
            "ETag: \"version-{}\"\r\n",
            if mode == 2 || (mode == 10 && count > 1) {
                2
            } else {
                1
            }
        ));
    }
    if mode == 8 {
        headers.push_str("Content-Type: text/html\r\n");
    }
    if mode == 6 {
        headers.push_str("Content-Encoding: gzip\r\n");
    }
    for value in disposition.lock().unwrap().iter() {
        headers.push_str(&format!("Content-Disposition: {value}\r\n"));
    }
    headers.push_str("\r\n");
    stream.write_all(headers.as_bytes()).await?;
    if count == 0 {
        return Ok(());
    }
    let body = &data[start..=end];
    let body = if mode == 4 && count > 1 {
        &body[..body.len() / 2]
    } else {
        body
    };
    for (index, chunk) in body.chunks(32 * 1024).enumerate() {
        if count > 1
            && let Some((after, gate)) = &body_gate
            && index * 32 * 1024 >= *after
        {
            // Hold a known byte boundary until the test releases it. A slow
            // observer must not let a pause fixture finish before cancellation.
            drop(gate.acquire().await.unwrap());
        }
        if mode == 9 && start >= data.len() / 2 {
            // Different bytes in later segments with the same ETag and size.
            stream
                .write_all(&chunk.iter().map(|b| b ^ 0xff).collect::<Vec<_>>())
                .await?;
        } else {
            stream.write_all(chunk).await?;
        }
        if slow && count > 1 {
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    }
    Ok(())
}
fn setup() -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    (temp, store)
}
fn enqueue(store: &Store, url: &str) -> Job {
    let mut job = Job::new(
        url,
        Some("fixture.bin"),
        store.paths.downloads.clone(),
        uuid::Uuid::new_v4().to_string(),
    )
    .unwrap();
    job.status = Status::Downloading;
    store.enqueue(&job).unwrap()
}
async fn run(store: &Store, job: Job) -> Job {
    let id = job.id.clone();
    engine::run(
        store.clone(),
        job,
        CancellationToken::new(),
        download_engine::bandwidth::Limiter::new(&store.bandwidth_limits().unwrap()).unwrap(),
    )
    .await
    .unwrap();
    store.get(&id).unwrap().unwrap()
}

// Store opens and reads SQLite synchronously. Keep polling off the test runtime
// thread, which also drives the loopback HTTP server and direct engine worker.
async fn read_job(store: &Store, id: &str) -> Result<Job, String> {
    let store = store.clone();
    let id = id.to_owned();
    tokio::task::spawn_blocking(move || {
        store
            .get(&id)
            .map_err(|error| format!("Reading job failed: {error:#}"))?
            .ok_or_else(|| "The observed download disappeared".into())
    })
    .await
    .map_err(|error| format!("Job reader stopped: {error}"))?
}
fn describe_job(job: &Job) -> String {
    format!(
        "{}: {:?}, {}/{} bytes, retry {:?}, error {:?}",
        job.name,
        job.status,
        job.downloaded,
        job.total
            .map_or_else(|| "unknown".into(), |n| n.to_string()),
        job.retry,
        job.error
    )
}

/// Under load (16 threads at 85-100% CPU, a release build writing to the same
/// disk) one scheduler tick took up to 18 s, one engine run up to 25 s, and one
/// Windows credential write 19 s. Waits fail only after this long without any
/// observable change, so slow progress passes and a stalled step still fails.
const PROGRESS_LIMIT: Duration = Duration::from_secs(60);

#[derive(Debug)]
struct Stalled;

type Progress = (
    serde_json::Value,
    Option<download_engine::transfer_activity::Phase>,
    Vec<download_engine::transfer_activity::SegmentSnapshot>,
);

/// Saved jobs, plus the live segment progress made between their checkpoints.
fn store_progress(store: &Store) -> Vec<Progress> {
    let jobs = store.list().unwrap();
    jobs.into_iter()
        .map(|job| {
            let phase = store.activity.get(&job.id);
            let segments = store.activity.segments(&job.id);
            (serde_json::to_value(&job).unwrap(), phase, segments)
        })
        .collect()
}

/// Like `tokio::time::timeout`, but the limit restarts whenever `store` changes.
async fn until_stalled<T>(
    store: &Store,
    future: impl std::future::Future<Output = T>,
) -> Result<T, Stalled> {
    tokio::pin!(future);
    let mut seen = store_progress(store);
    loop {
        tokio::select! {
            result = &mut future => return Ok(result),
            _ = tokio::time::sleep(PROGRESS_LIMIT) => {
                let now = store_progress(store);
                if now == seen {
                    return Err(Stalled);
                }
                seen = now;
            }
        }
    }
}

async fn pause_midway(store: &Store, job: Job, fixture: &Fixture) -> Job {
    try_pause_midway(store, job, fixture).await.unwrap()
}
async fn try_pause_midway(store: &Store, job: Job, fixture: &Fixture) -> Result<Job, String> {
    let token = CancellationToken::new();
    let child = token.clone();
    let worker_store = store.clone();
    let id = job.id.clone();
    let limiter =
        download_engine::bandwidth::Limiter::new(&store.bandwidth_limits().unwrap()).unwrap();
    let task = tokio::spawn(async move { engine::run(worker_store, job, child, limiter).await });
    let mut last = None;
    let observed = until_stalled(store, async {
        loop {
            let saved = read_job(store, &id).await?;
            let mut progress = saved.downloaded;
            if saved.plan.is_none() {
                // Single-stream progress checkpoints happen on arriving chunks.
                // A gated response may hold bytes on disk before the next save.
                match tokio::fs::metadata(store.paths.parts(&id).unwrap().join("whole.part")).await
                {
                    Ok(file) => progress = progress.max(file.len()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(format!("Reading partial download failed: {error}")),
                }
            }
            let failed = saved.status == Status::Failed || saved.retry.attempts > 0;
            last = Some(saved);
            if task.is_finished() {
                // The worker may have saved its outcome after the previous read.
                let saved = read_job(store, &id).await?;
                let error = format!(
                    "Worker stopped before the pause point; {}",
                    describe_job(&saved)
                );
                last = Some(saved);
                return Err(error);
            }
            if failed {
                return Err(format!(
                    "Worker failed before the pause point; {}",
                    describe_job(last.as_ref().unwrap())
                ));
            }
            if progress >= 128 * 1024 {
                return Ok::<_, String>(());
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    // Join even on failure, so the test never leaves a writer using its TempDir.
    token.cancel();
    let worker = task.await;
    fixture.release_body();
    let state = last
        .as_ref()
        .map_or_else(|| "no state read".into(), describe_job);
    let observed = observed.map_err(|_| {
        format!(
            "Partial progress stalled; {state}; requests={}; worker={worker:?}",
            fixture.requests.lock().unwrap().len(),
        )
    })?;
    worker
        .map_err(|error| format!("Download worker panicked: {error}; {state}"))?
        .map_err(|error| format!("Download worker failed: {error:#}; {state}"))?;
    observed?;
    let paused = read_job(store, &id).await?;
    assert_eq!(paused.status, Status::Paused);
    assert!(
        paused.downloaded > 0 && paused.downloaded < paused.total.unwrap(),
        "paused: {paused:?}"
    );
    Ok(paused)
}
#[tokio::test]
async fn pause_wait_reports_worker_failure_without_waiting_for_timeout() {
    let fixture = Fixture::new(256 * 1024, 7, false).await;
    let (_temp, store) = setup();
    let error = try_pause_midway(&store, enqueue(&store, &fixture.url), &fixture)
        .await
        .unwrap_err();
    assert!(error.starts_with("Worker "), "{error}");
    assert!(error.contains("503"), "{error}");
    assert!(error.contains("retry"), "{error}");
    assert_eq!(fixture.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn pause_wait_reports_worker_store_error_without_waiting_for_timeout() {
    let fixture = Fixture::new(256 * 1024, 0, false).await;
    let (_temp, store) = setup();
    let job = enqueue(&store, &fixture.url);
    rusqlite::Connection::open(store.paths.data.join("queue.sqlite3")).unwrap()
        .execute_batch("CREATE TRIGGER reject_update BEFORE UPDATE ON jobs BEGIN SELECT RAISE(FAIL, 'injected queue write failure'); END;").unwrap();
    let error = try_pause_midway(&store, job, &fixture).await.unwrap_err();
    assert!(error.starts_with("Download worker failed:"), "{error}");
    assert!(error.contains("injected queue write failure"), "{error}");
}

#[tokio::test]
async fn parallel_ranges_reconstruct_exact_bytes_and_preserve_collisions() {
    let fixture = Fixture::new(9 * 1024 * 1024, 0, false).await;
    let (_temp, store) = setup();
    std::fs::write(store.paths.downloads.join("fixture.bin"), b"original").unwrap();
    let job = run(&store, enqueue(&store, &fixture.url)).await;
    assert_eq!(job.status, Status::Complete, "{:?}", job.error);
    assert_eq!(job.plan.as_ref().unwrap().segments.len(), 4);
    assert_eq!(
        std::fs::read(job.saved_path.unwrap()).unwrap(),
        *fixture.data
    );
    assert_eq!(
        std::fs::read(store.paths.downloads.join("fixture.bin")).unwrap(),
        b"original"
    );
    assert_eq!(fixture.requests.lock().unwrap().len(), 5);
    assert!(!store.paths.parts(&job.id).unwrap().exists());
}
#[tokio::test]
async fn ignored_range_falls_back_to_single_download() {
    let fixture = Fixture::new(150000, 1, false).await;
    let (_temp, store) = setup();
    let job = run(&store, enqueue(&store, &fixture.url)).await;
    assert_eq!(job.status, Status::Complete, "{:?}", job.error);
    assert!(job.plan.is_none());
    assert_eq!(
        std::fs::read(job.saved_path.unwrap()).unwrap(),
        *fixture.data
    );
    assert_eq!(fixture.requests.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn pause_and_resume_after_store_reopen_preserves_all_bytes() {
    let fixture = Fixture::pausable_range(1024 * 1024, 0).await;
    let (_temp, store) = setup();
    let paused = pause_midway(&store, enqueue(&store, &fixture.url), &fixture).await;
    let prior = paused.downloaded;
    assert_eq!(
        prior,
        128 * 1024,
        "The response must stop at the controlled pause boundary"
    );
    let reopened = Store::open(store.paths.clone()).unwrap();
    let complete = run(&reopened, paused).await;
    assert_eq!(complete.status, Status::Complete, "{:?}", complete.error);
    assert_eq!(
        std::fs::read(complete.saved_path.unwrap()).unwrap(),
        *fixture.data
    );
    let wanted = format!("range: bytes={prior}-");
    assert!(
        fixture
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.to_ascii_lowercase().contains(&wanted))
    );
}
#[tokio::test]
async fn changed_file_is_rejected_on_resume() {
    let fixture = Fixture::pausable_range(1024 * 1024, 0).await;
    let (_temp, store) = setup();
    let paused = pause_midway(&store, enqueue(&store, &fixture.url), &fixture).await;
    fixture.mode.store(2, Ordering::SeqCst);
    let failed = run(&store, paused).await;
    assert_eq!(failed.status, Status::Failed);
    assert!(failed.error.unwrap().contains("remote file changed"));
    assert!(failed.saved_path.is_none());
}
#[tokio::test]
async fn non_resumable_pause_starts_over_without_appending() {
    let fixture = Fixture::pausable_range(1024 * 1024, 1).await;
    let (_temp, store) = setup();
    let paused = pause_midway(&store, enqueue(&store, &fixture.url), &fixture).await;
    assert_eq!(paused.downloaded, 128 * 1024);
    assert!(paused.plan.is_none());
    let complete = run(&store, paused).await;
    assert_eq!(complete.status, Status::Complete);
    assert_eq!(
        std::fs::read(complete.saved_path.unwrap()).unwrap(),
        *fixture.data
    );
}
#[tokio::test]
async fn invalid_ranges_truncated_bodies_and_http_errors_never_publish() {
    for mode in [3, 4, 6, 7] {
        let fixture = Fixture::new(300000, mode, false).await;
        let (_temp, store) = setup();
        let failed = run(&store, enqueue(&store, &fixture.url)).await;
        assert_eq!(
            failed.status,
            if matches!(mode, 4 | 7) {
                Status::Queued
            } else {
                Status::Failed
            },
            "mode {mode}"
        );
        assert_eq!(failed.retry.next_at.is_some(), matches!(mode, 4 | 7));
        assert!(failed.saved_path.is_none());
        assert!(!store.paths.downloads.join("fixture.bin").exists());
        assert!(failed.error.is_some());
    }
}
#[tokio::test]
async fn interrupted_segment_can_retry_from_its_actual_file_length() {
    let fixture = Fixture::new(300000, 4, false).await;
    let (_temp, store) = setup();
    let failed = run(&store, enqueue(&store, &fixture.url)).await;
    assert_eq!(failed.status, Status::Queued);
    assert_eq!(failed.retry.attempts, 1);
    assert!(failed.retry.next_at.is_some());
    assert!(failed.downloaded > 0);
    fixture.mode.store(0, Ordering::SeqCst);
    let complete = run(&store, failed).await;
    assert_eq!(complete.status, Status::Complete, "{:?}", complete.error);
    assert_eq!(
        std::fs::read(complete.saved_path.unwrap()).unwrap(),
        *fixture.data
    );
}
#[tokio::test]
async fn empty_file_and_no_validator_servers_are_supported() {
    for (size, mode) in [(0, 0), (10000, 5)] {
        let fixture = Fixture::new(size, mode, false).await;
        let (_temp, store) = setup();
        let complete = run(&store, enqueue(&store, &fixture.url)).await;
        assert_eq!(complete.status, Status::Complete, "{:?}", complete.error);
        assert!(complete.plan.is_none());
        assert_eq!(
            std::fs::read(complete.saved_path.unwrap()).unwrap(),
            *fixture.data
        );
    }
}
#[test]
fn queue_handoff_is_durable_and_idempotent() {
    let (_temp, store) = setup();
    let data = br#"{"action":"add","request_id":"same","url":"https://example.com/a.zip"}"#;
    let first = native::handle(&store, data);
    let second = native::handle(&store, data);
    assert!(first.ok);
    assert_eq!(first.job_id, second.job_id);
    let reopened = Store::open(store.paths.clone()).unwrap();
    assert_eq!(reopened.list().unwrap().len(), 1);
    assert!(
        !native::handle(
            &store,
            br#"{"action":"add","request_id":"same","url":"https://example.com/b"}"#
        )
        .ok
    );
    assert!(
        !native::handle(
            &store,
            br#"{"action":"add","request_id":"x","url":"file:///C:/a"}"#
        )
        .ok
    );
}
#[test]
fn recovery_pauses_active_jobs_and_keeps_queued_jobs() {
    let (_temp, store) = setup();
    let active = enqueue(&store, "https://example.com/active");
    let queued = Job::new(
        "https://example.com/queued",
        None,
        store.paths.downloads.clone(),
        "queued".into(),
    )
    .unwrap();
    store.enqueue(&queued).unwrap();
    store.recover().unwrap();
    assert_eq!(
        store.get(&active.id).unwrap().unwrap().status,
        Status::Paused
    );
    assert_eq!(
        store.get(&queued.id).unwrap().unwrap().status,
        Status::Queued
    );
}
#[test]
fn native_framing_handles_partial_reads_unicode_and_invalid_lengths() {
    let reply = native::Reply {
        capture_state: None,
        capabilities: vec![],
        job_ids: vec![],
        batch_options: None,
        preview_id: None,
        recovery: None,
        ok: true,
        version: 1,
        job_id: None,
        message: "Ready ✓ 文件".into(),
    };
    let mut output = Vec::new();
    native::write_frame(&mut output, &reply).unwrap();
    struct OneByte(Cursor<Vec<u8>>);
    impl std::io::Read for OneByte {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = buf.len().min(1);
            std::io::Read::read(&mut self.0, &mut buf[..n])
        }
    }
    let mut input = OneByte(Cursor::new(output));
    let parsed: native::Reply =
        serde_json::from_slice(&native::read_frame(&mut input).unwrap().unwrap()).unwrap();
    assert_eq!(parsed.message, reply.message);
    assert!(native::read_frame(&mut input).unwrap().is_none());
    for data in [
        vec![1],
        vec![0, 0, 0, 0],
        vec![1, 0, 0, 0],
        ((native::MAX_FRAME + 1) as u32).to_le_bytes().to_vec(),
    ] {
        assert!(native::read_frame(&mut Cursor::new(data)).is_err());
    }
}
#[test]
fn real_native_host_uses_clean_stdio_and_reports_bad_messages() {
    let temp = TempDir::new().unwrap();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_de-host"))
        .env("IDM_EX_HOME", temp.path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    {
        let input = child.stdin.as_mut().unwrap();
        for data in [
            br#"{"action":"ping"}"#.as_slice(),
            br#"{"action":"invalid"}"#.as_slice(),
        ] {
            input.write_all(&(data.len() as u32).to_le_bytes()).unwrap();
            input.write_all(data).unwrap();
        }
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    let mut cursor = Cursor::new(output.stdout);
    let a: native::Reply =
        serde_json::from_slice(&native::read_frame(&mut cursor).unwrap().unwrap()).unwrap();
    let b: native::Reply =
        serde_json::from_slice(&native::read_frame(&mut cursor).unwrap().unwrap()).unwrap();
    assert!(a.ok);
    assert!(!b.ok);
    assert!(native::read_frame(&mut cursor).unwrap().is_none());
}
#[test]
fn filenames_urls_and_ranges_reject_unsafe_inputs() {
    assert_eq!(safe_filename("CON.txt"), "_CON.txt");
    assert_eq!(safe_filename("LPT1"), "_LPT1");
    assert_eq!(safe_filename("COM¹.txt"), "_COM¹.txt");
    assert_eq!(safe_filename("CON .txt"), "_CON .txt");
    assert!(safe_filename(&"😀".repeat(160)).encode_utf16().count() <= 160);
    assert_eq!(safe_filename("../../a?.zip"), ".._.._a_.zip");
    assert_eq!(safe_filename(".."), "download");
    assert!(validate_url("https://user:pass@example.com").is_err());
    assert!(validate_url("file:///C:/a").is_err());
    assert_eq!(
        validate_url("https://example.com/a#x").unwrap().as_str(),
        "https://example.com/a"
    );
    for value in [
        "bytes */1",
        "bytes 2-1/5",
        "bytes 0-5/5",
        "items 0-1/3",
        "bytes 0-0/18446744073709551615",
    ] {
        assert!(engine::parse_content_range(value).is_err());
    }
}
#[tokio::test]
async fn scheduler_accepts_later_handoffs_and_shutdown_pauses_workers() {
    // Hold the transfer open until shutdown has paused it. Under load, Stop can
    // wait out a long scheduler tick, longer than any paced transfer takes.
    let fixture = Fixture::pausable_range(2 * 1024 * 1024, 0).await;
    let (_temp, store) = setup();
    let scheduler = Scheduler::start(store.clone()).unwrap();
    let job = Job::new(
        &fixture.url,
        None,
        store.paths.downloads.clone(),
        "later".into(),
    )
    .unwrap();
    store.enqueue(&job).unwrap();
    until_stalled(&store, async {
        loop {
            if store.get(&job.id).unwrap().unwrap().plan.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "Initial handoff stalled; job={:?}; requests={}; errors={:?}",
            store.get(&job.id).unwrap(),
            fixture.requests.lock().unwrap().len(),
            scheduler.errors.try_iter().collect::<Vec<_>>()
        )
    });
    // Drop must let cancellation settle before releasing ownership.
    tokio::task::spawn_blocking(move || drop(scheduler))
        .await
        .unwrap();
    assert_eq!(store.get(&job.id).unwrap().unwrap().status, Status::Paused);
    fixture.release_body();
    let next = Scheduler::start(store.clone()).unwrap();
    next.send(Command::Resume(job.id.clone())).unwrap();
    until_stalled(&store, async {
        loop {
            let job = store.get(&job.id).unwrap().unwrap();
            assert_ne!(job.status, Status::Failed, "{:?}", job.error);
            if job.status == Status::Complete {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "Resumed handoff stalled; job={:?}; requests={}; errors={:?}",
            store.get(&job.id).unwrap(),
            fixture.requests.lock().unwrap().len(),
            next.errors.try_iter().collect::<Vec<_>>()
        )
    });
    tokio::task::spawn_blocking(move || drop(next))
        .await
        .unwrap();
}

#[tokio::test]
async fn publication_recovery_does_not_truncate_or_duplicate_the_finished_file() {
    let (_temp, store) = setup();
    // Reproduce a process stopping between publication and the completion commit.
    let mut job = enqueue(&store, "http://127.0.0.1:1/no-network-needed");
    let payload = b"already safely published";
    job.downloaded = payload.len() as u64;
    job.total = Some(job.downloaded);
    let staging = job.directory.join(format!(".idm-ex-{}.assembling", job.id));
    let destination = job.directory.join(&job.name);
    std::fs::write(&staging, payload).unwrap();
    std::fs::hard_link(&staging, &destination).unwrap();
    job.saved_path = Some(destination.clone());
    store.save(&job).unwrap();
    let recovered = run(&store, job).await;
    assert_eq!(recovered.status, Status::Complete, "{:?}", recovered.error);
    assert_eq!(std::fs::read(&destination).unwrap(), payload);
    assert!(!staging.exists());
    assert!(!destination.with_file_name("fixture (1).bin").exists());
}

#[tokio::test]
async fn scheduler_recovers_a_failed_completion_commit_without_stuck_state_or_duplicate_file() {
    let fixture = Fixture::new(64 * 1024, 0, false).await;
    let (_temp, store) = setup();
    let mut job = enqueue(&store, &fixture.url);
    job.status = Status::Queued;
    store.save(&job).unwrap();
    let connection = rusqlite::Connection::open(store.paths.data.join("queue.sqlite3")).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_completion BEFORE UPDATE ON jobs
         WHEN json_extract(NEW.data, '$.status') = 'complete'
         BEGIN SELECT RAISE(FAIL, 'fixture completion commit failed'); END;",
        )
        .unwrap();
    let scheduler = Scheduler::start(store.clone()).unwrap();
    wait_until(&store, || {
        store.get(&job.id).unwrap().unwrap().status == Status::Failed
    })
    .await;
    let failed = store.get(&job.id).unwrap().unwrap();
    assert!(
        failed
            .error
            .as_ref()
            .unwrap()
            .contains("fixture completion commit failed")
    );
    let published = failed.saved_path.as_ref().unwrap();
    assert_eq!(std::fs::read(published).unwrap(), *fixture.data);
    let requests = fixture.requests.lock().unwrap().len();

    connection
        .execute_batch("DROP TRIGGER reject_completion")
        .unwrap();
    scheduler.send(Command::Resume(job.id.clone())).unwrap();
    wait_until(&store, || {
        store.get(&job.id).unwrap().unwrap().status == Status::Complete
    })
    .await;
    tokio::task::spawn_blocking(move || drop(scheduler))
        .await
        .unwrap();
    let completed = store.get(&job.id).unwrap().unwrap();
    assert_eq!(completed.saved_path, failed.saved_path);
    assert_eq!(fixture.requests.lock().unwrap().len(), requests);
    assert_eq!(std::fs::read(published).unwrap(), *fixture.data);
    assert_eq!(std::fs::read_dir(&job.directory).unwrap().count(), 1);
}

#[test]
fn real_native_host_adds_once_while_desktop_lock_is_held() {
    let temp = TempDir::new().unwrap();
    let paths = AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    };
    let store = Store::open(paths.clone()).unwrap();
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(paths.data.join("desktop.lock"))
        .unwrap();
    fs2::FileExt::lock_exclusive(&lock).unwrap();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_de-host"))
        .env("IDM_EX_HOME", temp.path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let data=br#"{"action":"add","request_id":"process-retry","url":"https://example.com/sent-from-browser.zip"}"#;
    for _ in 0..2 {
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(&(data.len() as u32).to_le_bytes())
            .unwrap();
        child.stdin.as_mut().unwrap().write_all(data).unwrap();
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    let mut cursor = Cursor::new(output.stdout);
    let first: native::Reply =
        serde_json::from_slice(&native::read_frame(&mut cursor).unwrap().unwrap()).unwrap();
    let retry: native::Reply =
        serde_json::from_slice(&native::read_frame(&mut cursor).unwrap().unwrap()).unwrap();
    assert!(first.ok, "{}", first.message);
    assert_eq!(first.message, "Added to the download queue");
    assert_eq!(first.job_id, retry.job_id);
    assert_eq!(store.list().unwrap().len(), 1);
    assert_eq!(store.list().unwrap()[0].status, Status::Queued);
}

async fn capture_request(store: &Store, request: serde_json::Value) -> native::Reply {
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        native::handle(&store, &serde_json::to_vec(&request).unwrap())
    })
    .await
    .unwrap()
}
fn prepare_request(server: &Fixture, id: &str) -> serde_json::Value {
    serde_json::json!({"action":"capture_prepare", "request_id":id, "url":server.url,
        "filename":"browser-name.bin", "size":server.data.len(), "mime":"application/octet-stream"})
}
#[tokio::test]
async fn capture_prepares_outside_queue_then_commits_once_with_url_free_receipt() {
    use download_engine::store::CaptureState;
    let server = Fixture::new(400_000, 0, false).await;
    let (_temp, store) = setup();
    let request = prepare_request(&server, "capture:durable");
    let prepared = capture_request(&store, request.clone()).await;
    assert!(prepared.ok, "{}", prepared.message);
    assert_eq!(prepared.capture_state, Some(CaptureState::Prepared));
    assert!(store.list().unwrap().is_empty());
    assert_eq!(
        capture_request(&store, request).await.job_id,
        prepared.job_id
    );
    assert_eq!(server.requests.lock().unwrap().len(), 1);
    let reopened = Store::open(store.paths.clone()).unwrap();
    let commit = serde_json::json!({"action":"capture_commit","request_id":"capture:durable"});
    let reply = capture_request(&reopened, commit.clone()).await;
    assert_eq!(reply.capture_state, Some(CaptureState::Committed));
    assert_eq!(
        reply.job_id,
        capture_request(&reopened, commit).await.job_id
    );
    assert_eq!(store.list().unwrap().len(), 1);
    let queued = store.get(reply.job_id.as_ref().unwrap()).unwrap().unwrap();
    assert_eq!(queued.status, Status::Queued);
    assert_eq!(queued.name, "browser-name.bin");
    assert_eq!(queued.expected_file.as_ref().unwrap().size, 400_000);
    let done = run(&store, queued).await;
    assert_eq!(done.status, Status::Complete, "{:?}", done.error);
    assert_eq!(
        std::fs::read(done.saved_path.unwrap()).unwrap(),
        *server.data
    );
    store.remove(&done.id).unwrap();
    let receipt = store.capture_status("capture:durable").unwrap();
    assert_eq!(receipt.state, CaptureState::Committed);
    assert_eq!(receipt.job_id.as_deref(), Some(done.id.as_str()));
    assert!(
        receipt.job.is_none(),
        "Terminal receipts must not retain signed URLs"
    );
    assert!(
        capture_request(
            &store,
            serde_json::json!({
        "action":"capture_commit","request_id":"capture:durable"})
        )
        .await
        .ok
    );
    assert!(
        store.list().unwrap().is_empty(),
        "A retry must not resurrect a removed job"
    );
}
#[tokio::test]
async fn capture_refuses_browser_only_or_mismatched_files() {
    for (mode, size, mime) in [
        (7, 20_000, "application/octet-stream"),
        (8, 20_000, "application/octet-stream"),
        (0, 20_001, "application/octet-stream"),
        (0, 20_000, "application/zip"),
        (6, 20_000, "application/octet-stream"),
        (0, 0, "application/octet-stream"),
        (0, 20_000, "text/html"),
    ] {
        let server = Fixture::new(20_000, mode, false).await;
        let (_temp, store) = setup();
        let mut request = prepare_request(&server, "capture:reject");
        request["size"] = size.into();
        request["mime"] = mime.into();
        let reply = capture_request(&store, request).await;
        assert!(
            !reply.ok,
            "Unexpected acceptance: mode={mode}, size={size}, mime={mime}"
        );
        assert!(store.list().unwrap().is_empty());
        assert_eq!(
            store.capture_status("capture:reject").unwrap().state,
            download_engine::store::CaptureState::Missing
        );
    }
}
#[tokio::test]
async fn captured_file_is_checked_again_when_the_engine_starts() {
    for changed_mode in [2, 8] {
        let server = Fixture::new(80_000, 0, false).await;
        let (_temp, store) = setup();
        assert!(
            capture_request(&store, prepare_request(&server, "capture:change"))
                .await
                .ok
        );
        store.commit_capture("capture:change").unwrap();
        server.mode.store(changed_mode, Ordering::SeqCst);
        let result = run(&store, store.list().unwrap().remove(0)).await;
        assert_eq!(result.status, Status::Failed);
        assert!(result.saved_path.is_none());
        assert!(
            std::fs::read_dir(&result.directory)
                .unwrap()
                .next()
                .is_none()
        );
    }
}
#[test]
fn capture_abort_blocks_late_prepare_and_is_atomic_with_commit() {
    use download_engine::store::CaptureState;
    let (_temp, store) = setup();
    let mut job = Job::new(
        "https://example.test/file.bin",
        None,
        store.paths.downloads.clone(),
        "capture:late".into(),
    )
    .unwrap();
    job.expected_file = Some(
        download_engine::capture::ExpectedFile::browser(100, "application/octet-stream").unwrap(),
    );
    store.abort_capture(&job.request_id).unwrap();
    assert!(store.prepare_capture(&job).is_err());
    assert!(store.commit_capture(&job.request_id).is_err());
    assert!(store.list().unwrap().is_empty());
    for i in 0..12 {
        job.id = uuid::Uuid::new_v4().to_string();
        job.request_id = format!("capture:race-{i}");
        store.prepare_capture(&job).unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let (s, id, b) = (store.clone(), job.request_id.clone(), barrier.clone());
        let commit = std::thread::spawn(move || {
            b.wait();
            s.commit_capture(&id)
        });
        barrier.wait();
        let aborted = store.abort_capture(&job.request_id).unwrap();
        let committed = commit.join().unwrap();
        match store.capture_status(&job.request_id).unwrap().state {
            CaptureState::Committed => {
                assert!(committed.is_ok());
                assert_eq!(aborted.state, CaptureState::Committed);
                assert!(store.get(&job.id).unwrap().is_some());
            }
            CaptureState::Aborted => {
                assert!(committed.is_err());
                assert!(store.get(&job.id).unwrap().is_none());
            }
            _ => panic!("Transaction left a partial handoff"),
        }
    }
}
#[test]
fn old_jobs_remain_compatible_and_invalid_capture_messages_are_rejected() {
    let (_temp, store) = setup();
    let job = Job::new(
        "https://example.test/file",
        None,
        store.paths.downloads.clone(),
        "old-job".into(),
    )
    .unwrap();
    let mut json = serde_json::to_value(&job).unwrap();
    json.as_object_mut().unwrap().remove("expected_file");
    assert!(
        serde_json::from_value::<Job>(json)
            .unwrap()
            .expected_file
            .is_none()
    );
    for request in [
        serde_json::json!({"action":"capture_commit","request_id":"manual-id"}),
        serde_json::json!({"action":"capture_abort","request_id":"capture:"}),
        serde_json::json!({"action":"add","request_id":"capture:reserved","url":job.url}),
        serde_json::json!({"action":"capture_status","request_id":"capture:valid","unknown":true}),
    ] {
        assert!(!native::handle(&store, &serde_json::to_vec(&request).unwrap()).ok);
    }
    assert!(
        native::handle(&store, br#"{"action":"ping"}"#)
            .capabilities
            .iter()
            .any(|v| v == "automatic_capture_v1")
    );
}
async fn host_exchange(
    exe: std::path::PathBuf,
    home: std::path::PathBuf,
    requests: Vec<serde_json::Value>,
) -> Vec<native::Reply> {
    tokio::task::spawn_blocking(move || {
        use std::process::{Command, Stdio};
        let mut child = Command::new(exe)
            .env("IDM_EX_HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        for request in requests {
            let bytes = serde_json::to_vec(&request).unwrap();
            input
                .write_all(&(bytes.len() as u32).to_le_bytes())
                .unwrap();
            input.write_all(&bytes).unwrap();
        }
        drop(input);
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut reader = Cursor::new(output.stdout);
        let mut replies = vec![];
        while let Some(frame) = native::read_frame(&mut reader).unwrap() {
            replies.push(serde_json::from_slice(&frame).unwrap());
        }
        replies
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn real_native_capture_commit_requires_a_ready_desktop() {
    let server = Fixture::new(50_000, 0, false).await;
    let (temp, store) = setup();
    let isolated_host = temp.path().join("de-host.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_de-host"), &isolated_host).unwrap();
    let replies = host_exchange(
        isolated_host.clone(),
        temp.path().to_path_buf(),
        vec![
            prepare_request(&server, "capture:host"),
            serde_json::json!({"action":"capture_commit","request_id":"capture:host"}),
        ],
    )
    .await;
    assert!(replies[0].ok);
    assert!(!replies[1].ok, "Missing desktop must not commit");
    assert!(store.list().unwrap().is_empty());
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(store.paths.data.join("desktop.lock"))
        .unwrap();
    fs2::FileExt::lock_exclusive(&lock).unwrap();
    let replies = host_exchange(
        isolated_host,
        temp.path().to_path_buf(),
        vec![
            serde_json::json!({"action":"capture_commit","request_id":"capture:host"}),
            serde_json::json!({"action":"capture_commit","request_id":"capture:host"}),
        ],
    )
    .await;
    assert!(replies.iter().all(|r| r.ok));
    assert_eq!(replies[0].job_id, replies[1].job_id);
    assert_eq!(store.list().unwrap().len(), 1);
}

#[track_caller]
fn wait_until(
    store: &Store,
    mut condition: impl FnMut() -> bool,
) -> impl std::future::Future<Output = ()> {
    let caller = std::panic::Location::caller();
    async move {
        until_stalled(store, async {
            while !condition() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!("Queue transition made no progress for {PROGRESS_LIMIT:?} at {caller}")
        });
    }
}

#[tokio::test]
async fn named_queues_pause_independently_and_resume_exact_bytes_in_order() {
    use download_engine::queue::MAIN_QUEUE;
    // Hold the transfer open until the queue pause applies; under load a
    // scheduler command can wait longer than any paced transfer takes.
    let fixture = Fixture::pausable_range(4 * 1024 * 1024, 0).await;
    let (_temp, store) = setup();
    let now = chrono::Utc::now().timestamp();
    store.schedule_queue(MAIN_QUEUE, now + 3600, now).unwrap();
    let q = store.create_queue("Files").unwrap();
    store.set_queue_limit(&q.id, 1).unwrap();
    let mut jobs = Vec::new();
    for name in ["first.bin", "second.bin"] {
        jobs.push(
            Job::new(
                &fixture.url,
                Some(name),
                store.paths.downloads.clone(),
                name.into(),
            )
            .unwrap(),
        );
    }
    store.enqueue_batch(&jobs, &q.id).unwrap();
    store.reorder_job(&jobs[1].id, true).unwrap();
    let unrelated = Job::new(
        &fixture.url,
        Some("main.bin"),
        store.paths.downloads.clone(),
        "main".into(),
    )
    .unwrap();
    store.enqueue(&unrelated).unwrap();
    let scheduler = Scheduler::start(store.clone()).unwrap();
    scheduler.send(Command::StartQueue(q.id.clone())).unwrap();
    wait_until(&store, || {
        store.get(&jobs[1].id).unwrap().unwrap().downloaded > 0
    })
    .await;
    assert_eq!(
        store.get(&jobs[0].id).unwrap().unwrap().status,
        Status::Queued
    );
    assert_eq!(
        store.get(&unrelated.id).unwrap().unwrap().status,
        Status::Queued
    );
    scheduler.send(Command::PauseQueue(q.id.clone())).unwrap();
    wait_until(&store, || {
        !store
            .queues()
            .unwrap()
            .iter()
            .find(|q1| q1.id == q.id)
            .unwrap()
            .running
            && store.get(&jobs[1].id).unwrap().unwrap().status == Status::Queued
    })
    .await;
    let held = store.get(&jobs[1].id).unwrap().unwrap();
    let progress = held.downloaded;
    assert!(progress > 0);
    // The held response would end in a network timeout and a retry instead.
    assert!(held.retry.is_default(), "{:?}", held.retry);
    fixture.release_body();
    scheduler.send(Command::Pause(jobs[0].id.clone())).unwrap();
    scheduler.send(Command::StartQueue(q.id.clone())).unwrap();
    wait_until(&store, || {
        store.get(&jobs[1].id).unwrap().unwrap().status == Status::Complete
    })
    .await;
    assert_eq!(
        store.get(&jobs[0].id).unwrap().unwrap().status,
        Status::Paused
    );
    assert_eq!(
        std::fs::read(store.get(&jobs[1].id).unwrap().unwrap().saved_path.unwrap()).unwrap(),
        *fixture.data
    );
    assert_eq!(
        store.get(&unrelated.id).unwrap().unwrap().status,
        Status::Queued
    );
    assert!(scheduler.errors.try_iter().collect::<Vec<_>>().is_empty());
    tokio::task::spawn_blocking(move || drop(scheduler))
        .await
        .unwrap();
}

#[tokio::test]
async fn recurring_windows_resume_exact_bytes_after_app_restart_and_automatic_pause() {
    use chrono::Timelike;
    use download_engine::{
        queue::MAIN_QUEUE,
        queue_schedule::{EVERY_DAY, RecurringSchedule, clock_label},
    };
    let now = chrono::Local::now();
    let minute = (now.hour() * 60 + now.minute()) as u16;
    let inside = RecurringSchedule::new(
        EVERY_DAY,
        &clock_label((minute + 1380) % 1440),
        &clock_label((minute + 60) % 1440),
    )
    .unwrap();
    let outside = RecurringSchedule::new(
        EVERY_DAY,
        &clock_label((minute + 180) % 1440),
        &clock_label((minute + 240) % 1440),
    )
    .unwrap();
    let fixture = Fixture::pausable_range(4 * 1024 * 1024, 0).await;
    let (_temp, store) = setup();
    store
        .schedule_recurring_queue(MAIN_QUEUE, &outside, now.timestamp())
        .unwrap();
    let job = Job::new(
        &fixture.url,
        Some("scheduled.bin"),
        store.paths.downloads.clone(),
        "scheduled-resume".into(),
    )
    .unwrap();
    store.enqueue(&job).unwrap();
    let mut manual = Job::new(
        &fixture.url,
        Some("manual.bin"),
        store.paths.downloads.clone(),
        "manual-pause".into(),
    )
    .unwrap();
    manual.status = Status::Paused;
    store.enqueue(&manual).unwrap();
    let scheduler = Scheduler::start(store.clone()).unwrap();
    scheduler
        .send(Command::ScheduleRecurringQueue(
            MAIN_QUEUE.into(),
            inside.clone(),
        ))
        .unwrap();
    wait_until(&store, || {
        store.get(&job.id).unwrap().unwrap().downloaded >= 128 * 1024
    })
    .await;
    // A real app shutdown cancels the HTTP workers and flushes their partial files.
    assert!(scheduler.errors.try_iter().collect::<Vec<_>>().is_empty());
    tokio::task::spawn_blocking(move || drop(scheduler))
        .await
        .unwrap();
    let paused = store.get(&job.id).unwrap().unwrap();
    assert_eq!(paused.status, Status::Paused);
    assert!(paused.downloaded > 0 && paused.downloaded < fixture.data.len() as u64);
    let reopened = Store::open(store.paths.clone()).unwrap();
    reopened.recover().unwrap();
    assert_eq!(
        reopened.get(&job.id).unwrap().unwrap().status,
        Status::Queued
    );
    assert_eq!(
        reopened.get(&manual.id).unwrap().unwrap().status,
        Status::Paused
    );
    let scheduler = Scheduler::start(reopened.clone()).unwrap();
    wait_until(&reopened, || {
        reopened.get(&job.id).unwrap().unwrap().downloaded > paused.downloaded
    })
    .await;
    // Editing the window to exclude now exercises the same stop/cancellation
    // path used by a timer boundary, without a minute-long wall-clock test.
    scheduler
        .send(Command::ScheduleRecurringQueue(MAIN_QUEUE.into(), outside))
        .unwrap();
    wait_until(&reopened, || {
        !reopened.queues().unwrap()[0].running
            && reopened.get(&job.id).unwrap().unwrap().status == Status::Queued
    })
    .await;
    let waiting = reopened.get(&job.id).unwrap().unwrap();
    assert!(waiting.downloaded > paused.downloaded);
    assert_eq!(
        reopened.get(&manual.id).unwrap().unwrap().status,
        Status::Paused
    );
    let plan = waiting.plan.as_ref().unwrap();
    let part_len = std::fs::metadata(
        reopened
            .paths
            .parts(&job.id)
            .unwrap()
            .join("segment-0.part"),
    )
    .unwrap()
    .len();
    assert!(part_len > 0 && part_len < plan.segments[0].end - plan.segments[0].start + 1);
    let expected_range = format!(
        "range: bytes={}-{}",
        plan.segments[0].start + part_len,
        plan.segments[0].end
    );
    fixture.requests.lock().unwrap().clear();
    fixture.release_body();
    scheduler
        .send(Command::ScheduleRecurringQueue(MAIN_QUEUE.into(), inside))
        .unwrap();
    wait_until(&reopened, || {
        reopened.get(&job.id).unwrap().unwrap().status == Status::Complete
    })
    .await;
    let complete = reopened.get(&job.id).unwrap().unwrap();
    assert_eq!(
        std::fs::read(complete.saved_path.unwrap()).unwrap(),
        *fixture.data
    );
    assert!(
        fixture
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.to_ascii_lowercase().contains(&expected_range))
    );
    assert_eq!(
        reopened.get(&manual.id).unwrap().unwrap().status,
        Status::Paused
    );
    assert!(reopened.queues().unwrap()[0].automatic());
    assert!(scheduler.errors.try_iter().collect::<Vec<_>>().is_empty());
    tokio::task::spawn_blocking(move || drop(scheduler))
        .await
        .unwrap();
}

#[tokio::test]
async fn scheduled_queue_starts_once_without_starting_early() {
    let fixture = Fixture::new(128 * 1024, 0, false).await;
    let (_temp, store) = setup();
    let q = store.create_queue("Scheduled").unwrap();
    let job = Job::new(
        &fixture.url,
        None,
        store.paths.downloads.clone(),
        "scheduled".into(),
    )
    .unwrap();
    store
        .enqueue_batch(std::slice::from_ref(&job), &q.id)
        .unwrap();
    let at = chrono::Utc::now().timestamp() + 3;
    store
        .schedule_queue(&q.id, at, chrono::Utc::now().timestamp())
        .unwrap();
    let scheduler = Scheduler::start(store.clone()).unwrap();
    while chrono::Utc::now().timestamp() < at {
        let current = store.get(&job.id).unwrap().unwrap();
        // Read the clock again so crossing the second boundary cannot make this assertion flaky.
        if chrono::Utc::now().timestamp() < at {
            assert_eq!(current.status, Status::Queued);
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    wait_until(&store, || {
        store.get(&job.id).unwrap().unwrap().status == Status::Complete
    })
    .await;
    assert_eq!(
        std::fs::read(store.get(&job.id).unwrap().unwrap().saved_path.unwrap()).unwrap(),
        *fixture.data
    );
    assert_eq!(
        store
            .queues()
            .unwrap()
            .iter()
            .find(|saved| saved.id == q.id)
            .unwrap()
            .scheduled_at,
        None
    );
    assert_eq!(store.apply_queue_schedules(at + 60).unwrap(), 0);
    tokio::task::spawn_blocking(move || drop(scheduler))
        .await
        .unwrap();
}

#[cfg(windows)]
#[tokio::test]
async fn security_downloads_keep_internet_origin_metadata() {
    for mode in [0, 1] {
        let fixture = Fixture::new(64000, mode, false).await;
        let (_temp, store) = setup();
        let url = format!("{}?token=fixture-secret", fixture.url);
        let finished = run(&store, enqueue(&store, &url)).await;
        assert_eq!(finished.status, Status::Complete, "{:?}", finished.error);
        let mut stream = finished.saved_path.unwrap().into_os_string();
        stream.push(":Zone.Identifier");
        let marker = std::fs::read_to_string(std::path::PathBuf::from(stream))
            .expect("Published Internet download must have Mark of the Web");
        assert!(marker.contains("[ZoneTransfer]") && marker.contains("ZoneId=3"));
        assert!(!marker.contains("fixture-secret"));
    }
}

#[tokio::test]
async fn security_stale_staging_link_never_overwrites_another_file() {
    let fixture = Fixture::new(64000, 0, false).await;
    let (_temp, store) = setup();
    let job = enqueue(&store, &fixture.url);
    let preserved = store.paths.downloads.join("renamed-previous-download.bin");
    let original = b"This previously downloaded file must survive a retry.";
    std::fs::write(&preserved, original).unwrap();
    let staging = job.directory.join(format!(".idm-ex-{}.assembling", job.id));
    // A crash after publication followed by a user rename can leave this link.
    std::fs::hard_link(&preserved, &staging).unwrap();
    let finished = run(&store, job).await;
    assert!(
        std::fs::read(&preserved).unwrap() == original,
        "The preserved file was overwritten"
    );
    assert_eq!(finished.status, Status::Complete, "{:?}", finished.error);
    assert_eq!(
        std::fs::read(finished.saved_path.unwrap()).unwrap(),
        *fixture.data
    );
}

#[tokio::test]
async fn security_restarting_single_download_does_not_truncate_link_target() {
    let fixture = Fixture::new(64000, 1, false).await;
    let (_temp, store) = setup();
    let job = enqueue(&store, &fixture.url);
    let preserved = store.paths.downloads.join("keep.bin");
    std::fs::write(&preserved, b"keep this exact content").unwrap();
    let root = store.paths.parts(&job.id).unwrap();
    std::fs::create_dir_all(&root).unwrap();
    std::fs::hard_link(&preserved, root.join("whole.part")).unwrap();
    let finished = run(&store, job).await;
    assert!(
        std::fs::read(&preserved).unwrap() == b"keep this exact content",
        "The preserved file was overwritten"
    );
    assert_eq!(finished.status, Status::Complete, "{:?}", finished.error);
}

#[cfg(windows)]
#[tokio::test]
async fn security_resuming_rejects_partial_files_linked_to_other_files() {
    let fixture = Fixture::new(64000, 0, false).await;
    let (_temp, store) = setup();
    let job = enqueue(&store, &fixture.url);
    let preserved = store.paths.downloads.join("keep.bin");
    std::fs::write(&preserved, b"keep this exact content").unwrap();
    let root = store.paths.parts(&job.id).unwrap();
    std::fs::create_dir_all(&root).unwrap();
    std::fs::hard_link(&preserved, root.join("segment-0.part")).unwrap();
    let finished = run(&store, job).await;
    assert!(
        std::fs::read(&preserved).unwrap() == b"keep this exact content",
        "The preserved file was overwritten"
    );
    assert_eq!(finished.status, Status::Failed);
    assert!(finished.error.unwrap().contains("linked"));
    assert!(!store.paths.downloads.join("fixture.bin").exists());
}

#[test]
fn security_filename_direction_controls_cannot_hide_extensions() {
    for control in [
        '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}', '\u{202e}', '\u{2066}', '\u{2067}',
        '\u{2068}', '\u{2069}', '\u{200e}', '\u{200f}', '\u{061c}',
    ] {
        let name = safe_filename(&format!("invoice{control}fdp.exe"));
        assert!(!name.contains(control));
        assert!(name.ends_with(".exe"));
    }
    assert_eq!(safe_filename("صور-旅行.zip"), "صور-旅行.zip");
}

#[cfg(windows)]
#[tokio::test]
async fn security_download_cache_junction_cannot_redirect_file_writes() {
    use std::os::windows::process::CommandExt;
    let fixture = Fixture::new(64000, 1, false).await;
    let (temp, store) = setup();
    let job = enqueue(&store, &fixture.url);
    let foreign = temp.path().join("foreign");
    std::fs::create_dir(&foreign).unwrap();
    let preserved = foreign.join("whole.part");
    std::fs::write(&preserved, b"unrelated file").unwrap();
    let windows = download_engine::paths::file_browser()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let output = std::process::Command::new(windows.join("System32").join("cmd.exe"))
        .args(["/d", "/c", "mklink", "/J"])
        .arg(store.paths.parts(&job.id).unwrap())
        .arg(&foreign)
        .creation_flags(0x08000000)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Cannot create isolated junction fixture: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let finished = run(&store, job).await;
    assert_eq!(finished.status, Status::Failed);
    assert!(finished.error.unwrap().contains("linked download cache"));
    assert_eq!(std::fs::read(preserved).unwrap(), b"unrelated file");
    assert!(!store.paths.downloads.join("fixture.bin").exists());
}

#[cfg(windows)]
#[test]
fn security_folder_browser_resolves_to_the_windows_directory() {
    let path = download_engine::paths::file_browser().unwrap();
    assert!(path.is_absolute());
    assert!(path.is_file());
    assert_eq!(path.file_name().unwrap(), "explorer.exe");
    assert_ne!(path.parent().unwrap(), std::env::current_dir().unwrap());
}

#[path = "support/link_recovery.rs"]
mod link_recovery;
#[path = "support/queue_start.rs"]
mod queue_start;

#[cfg(windows)]
#[path = "support/authentication.rs"]
mod authentication;

#[cfg(windows)]
#[path = "support/browser_login.rs"]
mod browser_login;

#[path = "support/retries.rs"]
mod retries;

#[path = "support/bandwidth.rs"]
mod bandwidth;

#[tokio::test]
async fn notifications_follow_new_terminal_attempts_across_queues_not_history() {
    use download_engine::notifications::Finished;
    let success = Fixture::new(32000, 0, false).await;
    let failure = Fixture::new(32000, 11, false).await; // Missing synthetic Basic credentials -> 401.
    let (_temp, store) = setup();
    let queue = store.create_queue("Background files").unwrap();
    let complete = Job::new(
        &success.url,
        Some("success.bin"),
        store.paths.downloads.clone(),
        "success".into(),
    )
    .unwrap();
    let failed = Job::new(
        &failure.url,
        Some("failure.bin"),
        store.paths.downloads.clone(),
        "failure".into(),
    )
    .unwrap();
    let mut historical = Job::new(
        &success.url,
        Some("old.bin"),
        store.paths.downloads.clone(),
        "old".into(),
    )
    .unwrap();
    historical.status = Status::Failed;
    store.enqueue(&historical).unwrap();
    store.enqueue(&complete).unwrap();
    store
        .enqueue_batch(std::slice::from_ref(&failed), &queue.id)
        .unwrap();
    store.set_queue_running(&queue.id, true).unwrap();
    let scheduler = Scheduler::start(store.clone()).unwrap();
    let mut events = Vec::new();
    wait_until(&store, || {
        events.extend(scheduler.finished.try_iter());
        events.len() == 2
    })
    .await;
    assert!(events.contains(&Finished {
        job: complete.id.clone(),
        status: Status::Complete
    }));
    assert!(events.contains(&Finished {
        job: failed.id.clone(),
        status: Status::Failed
    }));
    assert_eq!(
        std::fs::read(
            store
                .get(&complete.id)
                .unwrap()
                .unwrap()
                .saved_path
                .unwrap()
        )
        .unwrap(),
        *success.data
    );
    assert_eq!(
        store.get(&failed.id).unwrap().unwrap().status,
        Status::Failed
    );
    drop(scheduler);
    let reopened = Scheduler::start(Store::open(store.paths.clone()).unwrap()).unwrap();
    tokio::time::sleep(Duration::from_millis(1100)).await; // Observe two real scheduler ticks with only historical jobs.
    assert!(reopened.finished.try_recv().is_err());
}

#[tokio::test]
async fn retry_waits_and_user_pauses_stay_quiet_until_terminal_completion() {
    let fixture = Fixture::new(32000, 7, false).await;
    let (_temp, store) = setup();
    let job = Job::new(
        &fixture.url,
        None,
        store.paths.downloads.clone(),
        "retry-notification".into(),
    )
    .unwrap();
    store.enqueue(&job).unwrap();
    let scheduler = Scheduler::start(store.clone()).unwrap();
    wait_until(&store, || {
        store.get(&job.id).unwrap().unwrap().retry.next_at.is_some()
    })
    .await;
    assert!(scheduler.finished.try_recv().is_err());
    scheduler.send(Command::Pause(job.id.clone())).unwrap();
    wait_until(&store, || {
        store.get(&job.id).unwrap().unwrap().status == Status::Paused
    })
    .await;
    assert!(scheduler.finished.try_recv().is_err());
    fixture.mode.store(0, Ordering::SeqCst);
    scheduler.send(Command::Resume(job.id.clone())).unwrap();
    let mut events = Vec::new();
    wait_until(&store, || {
        events.extend(scheduler.finished.try_iter());
        !events.is_empty()
    })
    .await;
    assert_eq!(
        events,
        vec![download_engine::notifications::Finished {
            job: job.id,
            status: Status::Complete
        }]
    );
}

#[path = "support/categories.rs"]
mod categories;

#[tokio::test]
async fn sha256_verifies_single_and_segmented_before_publication() {
    use sha2::{Digest, Sha256};
    for (size, mode) in [(1024, 1), (9 * 1024 * 1024, 0)] {
        let (_root, store) = setup();
        let fixture = Fixture::new(size, mode, false).await;
        let expected = format!("{:x}", Sha256::digest(&*fixture.data));
        let mut job = enqueue(&store, &fixture.url);
        job.integrity.expected = Some(expected.clone());
        let job = run(&store, job).await;
        assert_eq!(job.status, Status::Complete, "{:?}", job.error);
        assert_eq!(job.integrity.actual.as_ref(), Some(&expected));
        assert_eq!(
            std::fs::read(job.saved_path.unwrap()).unwrap(),
            *fixture.data
        );
        let mut bad = enqueue(&store, &fixture.url);
        bad.integrity.expected = Some("0".repeat(64));
        let bad = run(&store, bad).await;
        assert_eq!(bad.status, Status::Failed);
        assert_eq!(bad.integrity.matches(), Some(false));
        assert!(bad.saved_path.is_none());
        assert!(bad.retry.next_at.is_none());
        assert!(!store.paths.downloads.join("fixture (1).bin").exists());
    }
}
#[tokio::test]
async fn sha256_rechecks_recovered_publication_and_preserves_history_on_manual_mismatch() {
    use download_engine::integrity;
    let (_root, store) = setup();
    let fixture = Fixture::new(512, 0, false).await;
    let mut job = run(&store, enqueue(&store, &fixture.url)).await;
    let path = job.saved_path.clone().unwrap();
    let cancel = CancellationToken::new();
    let mismatch = integrity::verify_completed(&store, &job, &"0".repeat(64), &cancel).unwrap();
    assert_eq!(mismatch.matches(), Some(false));
    assert!(path.exists());
    assert_eq!(
        store.get(&job.id).unwrap().unwrap().status,
        Status::Complete
    );
    let staging = job.directory.join(format!(".idm-ex-{}.assembling", job.id));
    std::fs::hard_link(&path, &staging).unwrap();
    job.status = Status::Downloading;
    job.integrity.expected = Some("0".repeat(64));
    let recovered = run(&store, job.clone()).await;
    assert_eq!(recovered.status, Status::Failed);
    assert!(fixture.requests.lock().unwrap().len() < 10);
    store.remove(&job.id).unwrap();
    job.status = Status::Complete;
    assert!(integrity::verify_completed(&store, &job, "", &cancel).is_err());
}

#[path = "support/server_filenames.rs"]
mod server_filenames;

#[path = "support/quotas.rs"]
mod quotas;

#[path = "support/http_response_validation.rs"]
mod http_response_validation;

#[cfg(windows)]
#[path = "support/storage_failures.rs"]
mod storage_failures;
