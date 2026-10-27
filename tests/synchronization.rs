use chrono::{Local, TimeZone};
use download_engine::{
    bandwidth::Limiter,
    engine,
    model::{Job, Status},
    notifications::Finished,
    paths::AppPaths,
    preferences::Preferences,
    queue::{DownloadQueue, MAIN_QUEUE},
    queue_schedule::{EVERY_DAY, RecurringSchedule},
    scheduler::{Command, Scheduler},
    store::Store,
    synchronization::Phase,
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Semaphore,
    task::{JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct Resource {
    bytes: Vec<u8>,
    tag: Option<String>,
    ranges: bool,
    status: Option<u16>,
    conflict: bool,
    mime: &'static str,
    gate: Option<Arc<Semaphore>>,
}
struct Server {
    url: String,
    resource: Arc<Mutex<Resource>>,
    requests: Arc<Mutex<Vec<String>>>,
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
        let url = format!("http://{}/file.bin", listener.local_addr().unwrap());
        let resource = Arc::new(Mutex::new(Resource {
            bytes: b"first version".to_vec(),
            tag: Some("\"v1\"".into()),
            ranges: true,
            status: None,
            conflict: false,
            mime: "application/octet-stream",
            gate: None,
        }));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (config, log) = (resource.clone(), requests.clone());
        let task = tokio::spawn(async move {
            let mut handlers = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (stream, _) = accepted.unwrap();
                        let (resource, requests) = (config.clone(), log.clone());
                        handlers.spawn(async move { let _ = serve(stream, resource, requests).await; });
                    },
                    _ = handlers.join_next(), if !handlers.is_empty() => {},
                }
            }
        });
        Self {
            url,
            resource,
            requests,
            task,
        }
    }
    fn change(&self, update: impl FnOnce(&mut Resource)) {
        update(&mut self.resource.lock().unwrap());
    }
}
async fn serve(
    mut stream: TcpStream,
    config: Arc<Mutex<Resource>>,
    log: Arc<Mutex<Vec<String>>>,
) -> std::io::Result<()> {
    let mut request = Vec::new();
    while !request.ends_with(b"\r\n\r\n") {
        let byte = stream.read_u8().await?;
        request.push(byte);
        assert!(request.len() < 16000);
    }
    let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
    log.lock().unwrap().push(request.clone());
    let resource = config.lock().unwrap().clone();
    let field = |name: &str| request.lines().find_map(|line| line.strip_prefix(name));
    let conditional = resource
        .tag
        .as_ref()
        .is_some_and(|tag| field("if-none-match: ") == Some(tag.as_str()));
    let mut status = resource
        .status
        .unwrap_or(if conditional { 304 } else { 200 });
    let mut body = resource.bytes.as_slice();
    let mut extra = String::new();
    if let Some(tag) = &resource.tag {
        extra += &format!(
            "ETag: {}\r\n",
            if resource.conflict && status == 304 {
                "\"wrong\""
            } else {
                tag
            }
        );
    }
    if status == 200
        && resource.ranges
        && let Some(range) = field("range: bytes=")
    {
        let (start, end) = range.split_once('-').unwrap();
        let (start, end) = (
            start.parse::<usize>().unwrap(),
            end.parse::<usize>().unwrap(),
        );
        if start >= body.len() || end >= body.len() {
            status = 416;
            body = &[];
        } else {
            status = 206;
            extra += &format!("Content-Range: bytes {start}-{end}/{}\r\n", body.len());
            body = &body[start..=end];
        }
    }
    if status != 200 && status != 206 {
        body = &[];
    }
    stream.write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nContent-Type: {}\r\n{extra}Connection: close\r\n\r\n", body.len(), resource.mime).as_bytes()).await?;
    let split = body.len().min(128 * 1024);
    stream.write_all(&body[..split]).await?;
    if body.len() > split {
        if let Some(gate) = resource.gate {
            gate.acquire().await.unwrap().forget();
        }
        stream.write_all(&body[split..]).await?;
    }
    stream.shutdown().await
}
fn setup() -> (tempfile::TempDir, Store, DownloadQueue) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    let queue = store.create_sync_queue("Current files").unwrap();
    (temp, store, queue)
}
fn add(store: &Store, queue: &DownloadQueue, url: &str) -> Job {
    let job = Job::new(
        url,
        Some("file.bin"),
        store.paths.downloads.clone(),
        uuid::Uuid::new_v4().to_string(),
    )
    .unwrap();
    store.enqueue_batch(&[job], &queue.id).unwrap().remove(0)
}
async fn run(store: &Store, mut job: Job, token: CancellationToken) -> Job {
    job.status = Status::Downloading;
    store.save(&job).unwrap();
    let id = job.id.clone();
    engine::run(store.clone(), job, token, Limiter::default())
        .await
        .unwrap();
    store.get(&id).unwrap().unwrap()
}
async fn first(store: &Store, queue: &DownloadQueue, server: &Server) -> Job {
    let job = run(
        store,
        add(store, queue, &server.url),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(job.status, Status::Complete, "{:?}", job.error);
    job
}
fn prepare(store: &Store, queue: &DownloadQueue, job: &Job) -> Job {
    store.set_queue_running(&queue.id, true).unwrap();
    let job = store.get(&job.id).unwrap().unwrap();
    assert_eq!(job.status, Status::Queued);
    job
}
async fn check(store: &Store, queue: &DownloadQueue, job: &Job) -> Job {
    run(store, prepare(store, queue, job), CancellationToken::new()).await
}
fn state(job: &Job, phase: Phase) {
    assert_eq!(job.status, Status::Complete, "{:?}", job.error);
    let sync = job.synchronization.as_ref().unwrap();
    assert_eq!(sync.phase, phase);
    assert!(sync.checked_at.is_some());
}

#[tokio::test]
async fn conditional_checks_keep_the_same_copy_and_changed_versions_publish_separately() {
    for ranges in [true, false] {
        let server = Server::new().await;
        server.change(|r| r.ranges = ranges);
        let (_temp, store, queue) = setup();
        let old = first(&store, &queue, &server).await;
        server.requests.lock().unwrap().clear();
        let same = check(&store, &queue, &old).await;
        state(&same, Phase::UpToDate);
        assert_eq!(same.saved_path, old.saved_path);
        let requests = server.requests.lock().unwrap().clone();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].contains("if-none-match: \"v1\""));
        assert!(
            Finished::from_job(&same)
                .unwrap()
                .notice(&same, Preferences::default())
                .is_none()
        );
        server.change(|r| {
            r.tag = Some("\"v2\"".into());
            r.bytes = b"second version is a different size".to_vec();
        });
        let new = check(&store, &queue, &same).await;
        state(&new, Phase::Updated);
        assert_ne!(new.saved_path, old.saved_path);
        assert_eq!(
            std::fs::read(old.saved_path.as_ref().unwrap()).unwrap(),
            b"first version"
        );
        assert_eq!(
            std::fs::read(new.saved_path.as_ref().unwrap()).unwrap(),
            b"second version is a different size"
        );
        let same = check(&store, &queue, &new).await;
        state(&same, Phase::UpToDate);
        assert_eq!(same.saved_path, new.saved_path);
        assert_eq!(
            std::fs::read_dir(&store.paths.downloads).unwrap().count(),
            2
        );
    }
}

#[tokio::test]
async fn absent_or_weak_tags_compare_actual_bytes_without_duplicate_files() {
    for tag in [None, Some("W/\"weak\"".into())] {
        let server = Server::new().await;
        server.change(|r| r.tag = tag);
        let (_temp, store, queue) = setup();
        let old = first(&store, &queue, &server).await;
        server.requests.lock().unwrap().clear();
        let same = check(&store, &queue, &old).await;
        state(&same, Phase::UpToDate);
        assert_eq!(same.saved_path, old.saved_path);
        assert!(
            !server
                .requests
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.contains("if-none-match"))
        );
        assert_eq!(
            std::fs::read_dir(&store.paths.downloads).unwrap().count(),
            1
        );
        server.change(|r| r.bytes = b"new bytes".to_vec());
        let new = check(&store, &queue, &same).await;
        state(&new, Phase::Updated);
        assert_eq!(
            std::fs::read(new.saved_path.unwrap()).unwrap(),
            b"new bytes"
        );
    }
}

#[tokio::test]
async fn a_new_tag_with_identical_bytes_updates_the_validator_without_publishing() {
    let server = Server::new().await;
    let (_temp, store, queue) = setup();
    let old = first(&store, &queue, &server).await;
    server.change(|r| r.tag = Some("\"v2\"".into()));
    let same = check(&store, &queue, &old).await;
    state(&same, Phase::UpToDate);
    assert_eq!(same.saved_path, old.saved_path);
    assert_eq!(
        same.remote.as_ref().unwrap().etag.as_deref(),
        Some("\"v2\"")
    );
    server.requests.lock().unwrap().clear();
    state(&check(&store, &queue, &same).await, Phase::UpToDate);
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn missing_or_edited_local_files_cannot_be_certified_by_a_304() {
    for missing in [false, true] {
        let server = Server::new().await;
        let (_temp, store, queue) = setup();
        let old = first(&store, &queue, &server).await;
        let path = old.saved_path.as_ref().unwrap();
        if missing {
            std::fs::remove_file(path).unwrap();
        } else {
            std::fs::write(path, b"user's edits").unwrap();
        }
        server.requests.lock().unwrap().clear();
        let new = check(&store, &queue, &old).await;
        state(&new, Phase::Updated);
        assert_eq!(
            std::fs::read(new.saved_path.as_ref().unwrap()).unwrap(),
            b"first version"
        );
        assert!(
            !server
                .requests
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.contains("if-none-match"))
        );
        if !missing {
            assert_eq!(std::fs::read(path).unwrap(), b"user's edits");
        }
    }
}

#[tokio::test]
async fn failed_checks_and_html_login_pages_preserve_the_previous_copy() {
    for scenario in [
        "unauthorized",
        "missing",
        "conflicting-tag",
        "html",
        "checksum",
    ] {
        let server = Server::new().await;
        let (_temp, store, queue) = setup();
        let mut old = first(&store, &queue, &server).await;
        if scenario == "checksum" {
            old.integrity.expected = old.integrity.actual.clone();
            store.save(&old).unwrap();
        }
        server.change(|r| match scenario {
            "unauthorized" => r.status = Some(401),
            "missing" => r.status = Some(404),
            "conflicting-tag" => r.conflict = true,
            "html" => {
                r.tag = Some("\"login\"".into());
                r.mime = "text/html";
                r.bytes = b"<html>log in</html>".to_vec();
            }
            _ => {
                r.tag = Some("\"changed\"".into());
                r.bytes = b"changed content".to_vec();
            }
        });
        let failed = check(&store, &queue, &old).await;
        assert_eq!(
            failed.status,
            Status::Failed,
            "{scenario}: {:?}",
            failed.error
        );
        assert_eq!(
            failed
                .synchronization
                .as_ref()
                .unwrap()
                .label(failed.status),
            "Check failed"
        );
        assert_eq!(
            std::fs::read(old.saved_path.as_ref().unwrap()).unwrap(),
            b"first version"
        );
        assert_eq!(
            failed.synchronization.as_ref().unwrap().previous.path,
            *old.saved_path.as_ref().unwrap()
        );
        assert!(!store.paths.downloads.join("file (1).bin").exists());
    }
}

#[tokio::test]
async fn browser_metadata_allows_a_new_size_but_keeps_the_file_type_guard() {
    let server = Server::new().await;
    let (_temp, store, queue) = setup();
    let mut old = first(&store, &queue, &server).await;
    old.expected_file = Some(
        download_engine::capture::ExpectedFile::browser(old.downloaded, "application/octet-stream")
            .unwrap(),
    );
    store.save(&old).unwrap();
    server.change(|r| {
        r.tag = Some("\"v2\"".into());
        r.bytes = b"much longer replacement version".to_vec();
    });
    state(&check(&store, &queue, &old).await, Phase::Updated);
}

fn local(value: &str) -> i64 {
    Local
        .from_local_datetime(
            &chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M").unwrap(),
        )
        .single()
        .unwrap()
        .timestamp()
}
#[tokio::test]
async fn each_schedule_window_checks_once_even_after_restart_or_reenable() {
    let server = Server::new().await;
    let (_temp, store, queue) = setup();
    let old = first(&store, &queue, &server).await;
    let schedule = RecurringSchedule::new(EVERY_DAY, "09:00", "11:00").unwrap();
    store
        .schedule_recurring_queue(&queue.id, &schedule, local("2026-10-12 08:00"))
        .unwrap();
    store
        .apply_queue_schedules(local("2026-10-12 09:30"))
        .unwrap();
    let checking = store.get(&old.id).unwrap().unwrap();
    assert_eq!(checking.status, Status::Queued);
    let same = run(&store, checking, CancellationToken::new()).await;
    state(&same, Phase::UpToDate);
    let reopened = Store::open(store.paths.clone()).unwrap();
    reopened
        .apply_queue_schedules(local("2026-10-12 10:00"))
        .unwrap();
    reopened
        .enable_recurring_schedule(&queue.id, local("2026-10-12 10:15"))
        .unwrap();
    assert_eq!(
        reopened.get(&old.id).unwrap().unwrap().status,
        Status::Complete
    );
    reopened
        .apply_queue_schedules(local("2026-10-13 12:00"))
        .unwrap();
    assert_eq!(
        reopened.get(&old.id).unwrap().unwrap().status,
        Status::Complete
    );
    reopened
        .apply_queue_schedules(local("2026-10-14 09:10"))
        .unwrap();
    assert_eq!(
        reopened.get(&old.id).unwrap().unwrap().status,
        Status::Queued
    );
}

#[tokio::test]
async fn one_time_and_manual_checks_preserve_ordinary_queues_and_individual_pause() {
    let server = Server::new().await;
    let (_temp, store, queue) = setup();
    let old = first(&store, &queue, &server).await;
    store.schedule_queue(&queue.id, 200, 100).unwrap();
    store.apply_queue_schedules(199).unwrap();
    assert_eq!(
        store.get(&old.id).unwrap().unwrap().status,
        Status::Complete
    );
    store.apply_queue_schedules(250).unwrap();
    let mut pending = store.get(&old.id).unwrap().unwrap();
    assert_eq!(pending.status, Status::Queued);
    pending.status = Status::Paused;
    store.save(&pending).unwrap();
    store.set_queue_running(&queue.id, true).unwrap();
    assert_eq!(store.get(&old.id).unwrap().unwrap().status, Status::Paused);
    let same = run(&store, pending, CancellationToken::new()).await;
    state(&same, Phase::UpToDate);
    store.apply_queue_schedules(300).unwrap();
    assert_eq!(
        store.get(&old.id).unwrap().unwrap().status,
        Status::Complete
    );
    store.move_job(&old.id, MAIN_QUEUE).unwrap();
    store.set_queue_running(MAIN_QUEUE, true).unwrap();
    assert_eq!(
        store.get(&old.id).unwrap().unwrap().status,
        Status::Complete
    );
    store.move_job(&old.id, &queue.id).unwrap();
    store.set_queue_running(&queue.id, true).unwrap();
    assert_eq!(store.get(&old.id).unwrap().unwrap().status, Status::Queued);
}

async fn wait_job(store: &Store, id: &str, predicate: impl Fn(&Job) -> bool) -> Job {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let job = store.get(id).unwrap().unwrap();
            if predicate(&job) {
                break job;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("Download state did not arrive")
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scheduler_pauses_an_update_and_resumes_after_restart_without_touching_the_old_file() {
    let server = Server::new().await;
    let (_temp, store, queue) = setup();
    let old = first(&store, &queue, &server).await;
    let gate = Arc::new(Semaphore::new(0));
    server.change(|r| {
        r.bytes = vec![7; 2 * 1024 * 1024];
        r.tag = Some("\"v2\"".into());
        r.gate = Some(gate.clone());
    });
    let scheduler = Scheduler::start(store.clone()).unwrap();
    scheduler
        .send(Command::StartQueue(queue.id.clone()))
        .unwrap();
    wait_job(&store, &old.id, |j| {
        j.synchronization
            .as_ref()
            .is_some_and(|s| s.phase == Phase::Updating)
            && j.downloaded >= 128 * 1024
    })
    .await;
    scheduler
        .send(Command::PauseQueue(queue.id.clone()))
        .unwrap();
    let paused = wait_job(&store, &old.id, |j| {
        j.status == Status::Queued && j.downloaded > 0
    })
    .await;
    drop(scheduler);
    let root = store.paths.parts(&old.id).unwrap();
    let saved: Vec<_> = paused
        .plan
        .as_ref()
        .unwrap()
        .segments
        .iter()
        .enumerate()
        .map(|(index, segment)| {
            let bytes = match std::fs::read(root.join(format!("segment-{index}.part"))) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                Err(error) => panic!("Cannot inspect paused update: {error}"),
            };
            assert!(bytes.iter().all(|byte| *byte == 7));
            (segment.start, segment.end, bytes.len() as u64)
        })
        .collect();
    assert_eq!(
        saved.iter().map(|(_, _, size)| size).sum::<u64>(),
        paused.downloaded
    );
    assert!(paused.downloaded >= 128 * 1024);
    let previous_requests = server.requests.lock().unwrap().len();
    assert_eq!(
        std::fs::read(old.saved_path.as_ref().unwrap()).unwrap(),
        b"first version"
    );
    server.change(|r| r.gate = None);
    gate.add_permits(10);
    let reopened = Store::open(store.paths.clone()).unwrap();
    let scheduler = Scheduler::start(reopened.clone()).unwrap();
    scheduler
        .send(Command::StartQueue(queue.id.clone()))
        .unwrap();
    let new = wait_job(&reopened, &old.id, |j| j.status == Status::Complete).await;
    state(&new, Phase::Updated);
    assert_eq!(
        std::fs::read(new.saved_path.unwrap()).unwrap(),
        vec![7; 2 * 1024 * 1024]
    );
    assert_eq!(
        std::fs::read(old.saved_path.unwrap()).unwrap(),
        b"first version"
    );
    let requests = server.requests.lock().unwrap();
    for (start, end, size) in saved.into_iter().filter(|(_, _, size)| *size > 0) {
        assert!(requests[previous_requests..].iter().any(|request| {
            request.to_ascii_lowercase().contains(&format!(
                "range: bytes={}-{}\r\n",
                start + size,
                end
            ))
        }));
    }
    drop(requests);
    drop(scheduler);
}

#[tokio::test]
async fn crash_after_atomic_publication_recovers_the_new_copy_without_rechecking_or_overwriting() {
    let server = Server::new().await;
    let (_temp, store, queue) = setup();
    let old = first(&store, &queue, &server).await;
    let mut pending = prepare(&store, &queue, &old);
    pending.synchronization.as_mut().unwrap().phase = Phase::Updating;
    pending.status = Status::Downloading;
    pending.plan = None;
    pending.downloaded = 9;
    pending.total = Some(9);
    pending.saved_path = Some(store.paths.downloads.join("file (1).bin"));
    let staging = pending
        .directory
        .join(format!(".idm-ex-{}.assembling", pending.id));
    std::fs::write(&staging, b"new bytes").unwrap();
    std::fs::hard_link(&staging, pending.saved_path.as_ref().unwrap()).unwrap();
    store.save(&pending).unwrap();
    let reopened = Store::open(store.paths.clone()).unwrap();
    reopened.recover().unwrap();
    server.requests.lock().unwrap().clear();
    let new = run(
        &reopened,
        reopened.get(&old.id).unwrap().unwrap(),
        CancellationToken::new(),
    )
    .await;
    state(&new, Phase::Updated);
    assert!(server.requests.lock().unwrap().is_empty());
    assert_eq!(
        std::fs::read(new.saved_path.unwrap()).unwrap(),
        b"new bytes"
    );
    assert_eq!(
        std::fs::read(old.saved_path.unwrap()).unwrap(),
        b"first version"
    );
    assert!(!staging.exists());
}

#[tokio::test]
async fn cancellation_during_a_check_can_resume_and_legacy_completed_jobs_can_be_synchronized() {
    let server = Server::new().await;
    let (_temp, store, queue) = setup();
    let mut old = first(&store, &queue, &server).await;
    // A history entry from before remote metadata and checksums were recorded.
    old.remote = None;
    old.integrity.actual = None;
    store.save(&old).unwrap();
    let pending = prepare(&store, &queue, &old);
    let cancel = CancellationToken::new();
    cancel.cancel();
    let paused = run(&store, pending, cancel).await;
    assert_eq!(paused.status, Status::Paused);
    assert_eq!(
        std::fs::read(old.saved_path.as_ref().unwrap()).unwrap(),
        b"first version"
    );
    let same = run(&store, paused, CancellationToken::new()).await;
    state(&same, Phase::UpToDate);
    assert_eq!(same.saved_path, old.saved_path);
    assert!(same.integrity.actual.is_some());
}
