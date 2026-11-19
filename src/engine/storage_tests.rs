//! Per-path, test-only write faults. They never exist in shipped binaries.
//! This exercises a full-volume error without filling the user's filesystem.
use super::*;
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

static FAULTS: OnceLock<Mutex<HashMap<PathBuf, usize>>> = OnceLock::new();
fn faults() -> &'static Mutex<HashMap<PathBuf, usize>> {
    FAULTS.get_or_init(Default::default)
}
pub(super) fn copy_fault_requested(path: &Path) -> bool {
    faults().lock().unwrap().contains_key(path)
}
pub(super) fn before_write(path: &Path, bytes: usize) -> std::io::Result<()> {
    let mut faults = faults().lock().unwrap();
    if let Some(remaining) = faults.get_mut(path) {
        if bytes > *remaining {
            #[cfg(windows)]
            let code = 112; // ERROR_DISK_FULL
            #[cfg(not(windows))]
            let code = 28; // ENOSPC
            return Err(std::io::Error::from_raw_os_error(code));
        }
        *remaining -= bytes;
    }
    Ok(())
}
struct FullDisk(PathBuf);
impl FullDisk {
    fn after(path: PathBuf, bytes: usize) -> Self {
        assert!(
            faults()
                .lock()
                .unwrap()
                .insert(path.clone(), bytes)
                .is_none()
        );
        Self(path)
    }
}
impl Drop for FullDisk {
    fn drop(&mut self) {
        faults().lock().unwrap().remove(&self.0);
    }
}
struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Failure {
    Single,
    Segment,
    Assembly,
    CompletedCopy,
    ChangedCopy,
}

async fn check_disk_full(stage: Failure) {
    use Failure::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::net::TcpListener;
    let temp = tempfile::TempDir::new().unwrap();
    let whole = matches!(stage, CompletedCopy | ChangedCopy);
    let other = std::env::var_os("DOWNLOAD_ENGINE_TEST_OTHER_VOLUME")
        .filter(|_| whole)
        .map(|folder| tempfile::tempdir_in(folder).unwrap());
    let store = Store::open(crate::paths::AppPaths {
        data: temp.path().join("data"),
        downloads: other
            .as_ref()
            .map_or_else(|| temp.path().join("downloads"), |t| t.path().to_owned()),
    })
    .unwrap();
    let original = store.connection_settings().unwrap();
    let mut changed = original.clone();
    changed.default_connections = if stage == Assembly { 4 } else { 1 };
    store.save_connection_settings(&original, changed).unwrap();
    let payload = std::sync::Arc::new(
        (0..if stage == Assembly {
            8 * 1024 * 1024
        } else {
            2 * 1024 * 1024
        })
            .map(|i| (i % 251) as u8)
            .collect::<Vec<_>>(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file.bin", listener.local_addr().unwrap());
    let data = payload.clone();
    let ranged = matches!(stage, Segment | Assembly);
    let requests = std::sync::Arc::new(AtomicUsize::new(0));
    let counted = requests.clone();
    let server = Server(tokio::spawn(async move {
        let mut tasks = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                connection = listener.accept() => {
                    let (mut socket, _) = connection.unwrap();
                    counted.fetch_add(1, Ordering::SeqCst);
                    let data = data.clone();
                    tasks.spawn(async move {
                        let mut head = Vec::new();
                        let mut byte = [0];
                        while !head.ends_with(b"\r\n\r\n") && head.len() < 32768 {
                            socket.read_exact(&mut byte).await?;
                            head.push(byte[0]);
                        }
                        let header = String::from_utf8(head).unwrap().to_ascii_lowercase();
                        let range = header.lines().find_map(|line| line.strip_prefix("range: bytes="));
                        let (start, end) = if ranged {
                            range.map(|value| {
                                let (start, end) = value.split_once('-').unwrap();
                                (start.parse::<usize>().unwrap(), end.parse::<usize>().unwrap())
                            }).unwrap_or((0, data.len() - 1))
                        } else {
                            (0, data.len() - 1)
                        };
                        let status = if ranged { "206 Partial Content" } else { "200 OK" };
                        let mut response = format!(
                            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\nETag: \"disk-fixture\"\r\n",
                            end - start + 1,
                        );
                        if ranged {
                            response.push_str(&format!("Content-Range: bytes {start}-{end}/{}\r\n", data.len()));
                        }
                        response.push_str("\r\n");
                        socket.write_all(response.as_bytes()).await?;
                        socket.write_all(&data[start..=end]).await
                    });
                },
                _ = tasks.join_next(), if !tasks.is_empty() => {}
            }
        }
    }));
    let mut job = Job::new(
        &url,
        Some("file.bin"),
        store.paths.downloads.clone(),
        uuid::Uuid::new_v4().to_string(),
    )
    .unwrap();
    job.status = Status::Downloading;
    let job = store.enqueue(&job).unwrap();
    let partial = store.paths.parts(&job.id).unwrap();
    let target = match stage {
        Single => partial.join("whole.part"),
        Segment => partial.join("segment-0.part"),
        _ => job.directory.join(format!(".idm-ex-{}.assembling", job.id)),
    };
    let fault = FullDisk::after(target.clone(), 512 * 1024);
    run(
        store.clone(),
        job.clone(),
        CancellationToken::new(),
        Limiter::new(&store.bandwidth_limits().unwrap()).unwrap(),
    )
    .await
    .unwrap();
    let mut failed = store.get(&job.id).unwrap().unwrap();
    assert_eq!(
        failed.status,
        if stage == Single {
            Status::Paused
        } else {
            Status::Queued
        },
        "{failed:?}"
    );
    let wait = failed.disk_wait.as_ref().unwrap();
    assert_eq!(
        wait.requirements[0].location,
        if stage == Assembly || whole {
            crate::disk_space::Location::Destination
        } else {
            crate::disk_space::Location::Temporary
        }
    );
    assert!(!wait.ready(&store, &failed, wait.check_after - 1));
    assert!(wait.ready(&store, &failed, wait.check_after));
    assert_eq!(failed.retry.attempts, 0);
    assert!(failed.retry.next_at.is_none());
    assert!(failed.saved_path.is_none());
    assert!(!job.directory.join(&job.name).exists());
    assert!(target.metadata().unwrap().len() > 0);
    assert!(target.metadata().unwrap().len() <= 512 * 1024);
    assert!(failed.error.as_deref().unwrap().contains("disk space"));
    if stage == Assembly || whole {
        assert_eq!(failed.downloaded, payload.len() as u64);
    }
    assert_eq!(failed.received_sha256.is_some(), whole);
    let server = if whole {
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        let source = partial.join("whole.part");
        assert_eq!(std::fs::read(&source).unwrap(), *payload);
        if other.is_some() {
            let src = std::fs::File::open(&source).unwrap();
            let dst = download_file::guard_download_parent(&target).unwrap();
            assert!(!download_file::same_volume(&src, &dst).unwrap());
        }
        if stage == ChangedCopy {
            use std::io::Write;
            std::fs::OpenOptions::new()
                .write(true)
                .open(&source)
                .unwrap()
                .write_all(b"x")
                .unwrap();
        }
        drop(server);
        None
    } else {
        Some(server)
    };
    drop(fault);
    // Bring the next probe forward without sleeping for a real low-space hold.
    failed.disk_wait.as_mut().unwrap().check_after = 0;
    store.save(&failed).unwrap();
    let store = Store::open(store.paths.clone()).unwrap();
    let scheduler = crate::scheduler::Scheduler::start(store.clone()).unwrap();
    if stage == Single {
        scheduler
            .send(crate::scheduler::Command::Resume(job.id.clone()))
            .unwrap();
    }
    let recovered = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let current = store.get(&job.id).unwrap().unwrap();
            if stage == ChangedCopy && current.status == Status::Failed {
                break current;
            }
            assert_ne!(current.status, Status::Failed, "{:?}", current.error);
            if current.status == Status::Complete && !partial.exists() {
                break current;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("scheduler did not finish the recovered download");
    drop(scheduler);
    drop(server);
    if whole {
        assert_eq!(requests.load(Ordering::SeqCst), 1);
    }
    if stage == ChangedCopy {
        assert!(
            recovered
                .error
                .unwrap()
                .contains("Cached HTTP file changed")
        );
        assert!(!job.directory.join(&job.name).exists());
        return;
    }
    assert_eq!(recovered.status, Status::Complete, "{:?}", recovered.error);
    assert!(recovered.received_sha256.is_none());
    assert_eq!(
        std::fs::read(recovered.saved_path.unwrap()).unwrap(),
        *payload
    );
    assert_eq!(store.list().unwrap().len(), 1);
    assert!(!partial.exists());
}

#[tokio::test]
async fn disk_full_during_segment_retains_bytes_and_resumes() {
    check_disk_full(Failure::Segment).await;
}
#[tokio::test]
async fn disk_full_during_single_stream_restarts_safely() {
    check_disk_full(Failure::Single).await;
}
#[tokio::test]
async fn disk_full_during_assembly_keeps_parts_for_retry() {
    check_disk_full(Failure::Assembly).await;
}

#[tokio::test]
async fn disk_full_after_http_eof_retries_the_copy_with_the_origin_offline() {
    check_disk_full(Failure::CompletedCopy).await;
}

#[tokio::test]
async fn disk_full_after_http_eof_rejects_a_changed_cache_file() {
    check_disk_full(Failure::ChangedCopy).await;
}

#[cfg(windows)]
#[tokio::test]
async fn queued_destination_is_rechecked_before_any_http_request() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(crate::paths::AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    let folder = temp.path().join("selected");
    std::fs::create_dir(&folder).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let job = store
        .new_download(&url, None, Some(&folder), "changed-folder".into())
        .unwrap();
    store.enqueue(&job).unwrap();
    std::fs::remove_dir(&folder).unwrap();
    std::fs::write(&folder, b"keep").unwrap();
    let limiter = Limiter::new(&store.bandwidth_limits().unwrap()).unwrap();
    tokio::select! {
        result = run(store.clone(), job.clone(), CancellationToken::new(), limiter) => result.unwrap(),
        _ = listener.accept() => panic!("HTTP request started before checking the destination"),
    }
    let failed = store.get(&job.id).unwrap().unwrap();
    assert_eq!(failed.status, Status::Failed);
    assert!(failed.error.unwrap().contains("not a folder"));
    assert_eq!(std::fs::read(folder).unwrap(), b"keep");
}
