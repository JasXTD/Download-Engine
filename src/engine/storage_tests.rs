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

async fn check_disk_full(stage: &str) {
    use tokio::net::TcpListener;
    let temp = tempfile::TempDir::new().unwrap();
    let store = Store::open(crate::paths::AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    let original = store.connection_settings().unwrap();
    let mut changed = original.clone();
    changed.default_connections = 1;
    store.save_connection_settings(&original, changed).unwrap();
    let payload = std::sync::Arc::new(
        (0..2 * 1024 * 1024)
            .map(|i| (i % 251) as u8)
            .collect::<Vec<_>>(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file.bin", listener.local_addr().unwrap());
    let data = payload.clone();
    let ranged = stage != "single";
    let _server = Server(tokio::spawn(async move {
        let mut tasks = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                connection = listener.accept() => {
                    let (mut socket, _) = connection.unwrap();
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
        "single" => partial.join("whole.part"),
        "segment" => partial.join("segment-0.part"),
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
    let failed = store.get(&job.id).unwrap().unwrap();
    assert_eq!(failed.status, Status::Failed, "{failed:?}");
    assert_eq!(failed.retry.attempts, 0);
    assert!(failed.retry.next_at.is_none());
    assert!(failed.saved_path.is_none());
    assert!(!job.directory.join(&job.name).exists());
    assert!(target.metadata().unwrap().len() > 0);
    assert!(target.metadata().unwrap().len() <= 512 * 1024);
    #[cfg(windows)]
    assert!(
        failed.error.as_ref().unwrap().contains("112"),
        "{:?}",
        failed.error
    );
    if stage == "assembly" {
        assert_eq!(failed.downloaded, payload.len() as u64);
    }
    drop(fault);
    run(
        store.clone(),
        failed,
        CancellationToken::new(),
        Limiter::new(&store.bandwidth_limits().unwrap()).unwrap(),
    )
    .await
    .unwrap();
    let recovered = store.get(&job.id).unwrap().unwrap();
    assert_eq!(recovered.status, Status::Complete, "{:?}", recovered.error);
    assert_eq!(
        std::fs::read(recovered.saved_path.unwrap()).unwrap(),
        *payload
    );
    assert_eq!(store.list().unwrap().len(), 1);
    assert!(!partial.exists());
}

#[tokio::test]
async fn disk_full_during_segment_retains_bytes_and_resumes() {
    check_disk_full("segment").await;
}
#[tokio::test]
async fn disk_full_during_single_stream_restarts_safely() {
    check_disk_full("single").await;
}
#[tokio::test]
async fn disk_full_during_assembly_keeps_parts_for_retry() {
    check_disk_full("assembly").await;
}
