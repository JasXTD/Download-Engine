//! Actual Windows sharing, read-only, ACL and unavailable-destination failures.
use super::*;
use std::{
    fs::OpenOptions, os::windows::fs::OpenOptionsExt, path::PathBuf, process::Command as Process,
};

fn parts(store: &Store, job: &Job) -> Vec<(PathBuf, Vec<u8>)> {
    std::fs::read_dir(store.paths.parts(&job.id).unwrap())
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect()
}
fn assert_failure(job: &Job) {
    assert_eq!(job.status, Status::Failed, "{job:?}");
    assert!(job.error.as_ref().is_some_and(|error| !error.is_empty()));
    assert!(
        job.retry.next_at.is_none(),
        "Storage failures must not consume network retries"
    );
}
async fn recover(store: &Store, job: Job, fixture: &Fixture) -> Job {
    let recovered = run(store, job).await;
    assert_eq!(recovered.status, Status::Complete, "{:?}", recovered.error);
    assert_eq!(
        std::fs::read(recovered.saved_path.as_ref().unwrap()).unwrap(),
        *fixture.data
    );
    assert_eq!(store.list().unwrap().len(), 1);
    assert!(!store.paths.parts(&recovered.id).unwrap().exists());
    recovered
}

#[tokio::test]
async fn locked_partial_file_preserves_progress_and_recovers_after_unlock() {
    let fixture = Fixture::pausable_range(2 * 1024 * 1024, 0).await;
    let (_temp, store) = setup();
    let paused = pause_midway(&store, enqueue(&store, &fixture.url), &fixture).await;
    let saved = parts(&store, &paused);
    let locked = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&saved[0].0)
        .unwrap();
    let failed = run(&store, paused).await;
    assert_failure(&failed);
    drop(locked);
    for (path, prefix) in saved {
        assert!(std::fs::read(path).unwrap().starts_with(&prefix));
    }
    recover(&store, failed, &fixture).await;
}

#[tokio::test]
async fn readonly_partial_file_is_not_deleted_and_recovers_after_permission_restore() {
    let fixture = Fixture::pausable_range(2 * 1024 * 1024, 0).await;
    let (_temp, store) = setup();
    let paused = pause_midway(&store, enqueue(&store, &fixture.url), &fixture).await;
    let saved = parts(&store, &paused);
    let original = std::fs::metadata(&saved[0].0).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_readonly(true);
    std::fs::set_permissions(&saved[0].0, readonly).unwrap();
    let failed = run(&store, paused).await;
    std::fs::set_permissions(&saved[0].0, original).unwrap();
    assert_failure(&failed);
    for (path, prefix) in saved {
        assert!(std::fs::read(path).unwrap().starts_with(&prefix));
    }
    recover(&store, failed, &fixture).await;
}

#[tokio::test]
async fn locked_assembly_file_keeps_complete_parts_and_never_overwrites_existing_download() {
    let fixture = Fixture::new(2 * 1024 * 1024, 0, false).await;
    let (_temp, store) = setup();
    let job = enqueue(&store, &fixture.url);
    let destination = job.directory.join(&job.name);
    std::fs::write(&destination, b"Keep this existing file").unwrap();
    let staging = job.directory.join(format!(".idm-ex-{}.assembling", job.id));
    let mut locked = OpenOptions::new()
        .write(true)
        .create_new(true)
        .share_mode(0)
        .open(&staging)
        .unwrap();
    locked.write_all(b"Locked assembly sentinel").unwrap();
    let failed = run(&store, job).await;
    assert_failure(&failed);
    assert_eq!(failed.downloaded, fixture.data.len() as u64);
    let request_count = fixture.requests.lock().unwrap().len();
    drop(locked);
    assert_eq!(
        std::fs::read(&staging).unwrap(),
        b"Locked assembly sentinel"
    );
    let recovered = recover(&store, failed, &fixture).await;
    assert_ne!(recovered.saved_path.as_ref().unwrap(), &destination);
    assert_eq!(
        std::fs::read(destination).unwrap(),
        b"Keep this existing file"
    );
    // Recovery probes the validator but does not redownload the saved body.
    assert_eq!(fixture.requests.lock().unwrap().len(), request_count + 1);
}

#[tokio::test]
async fn unavailable_destination_keeps_parts_until_the_folder_returns() {
    let fixture = Fixture::pausable_range(2 * 1024 * 1024, 0).await;
    let (temp, store) = setup();
    let paused = pause_midway(&store, enqueue(&store, &fixture.url), &fixture).await;
    let saved = parts(&store, &paused);
    let offline = temp.path().join("destination-offline");
    std::fs::rename(&store.paths.downloads, &offline).unwrap();
    std::fs::write(&store.paths.downloads, b"Unavailable folder sentinel").unwrap();
    let failed = run(&store, paused).await;
    assert_failure(&failed);
    assert_eq!(
        std::fs::read(&store.paths.downloads).unwrap(),
        b"Unavailable folder sentinel"
    );
    for (path, prefix) in saved {
        assert_eq!(std::fs::read(path).unwrap(), prefix);
    }
    std::fs::remove_file(&store.paths.downloads).unwrap();
    std::fs::rename(offline, &store.paths.downloads).unwrap();
    recover(&store, failed, &fixture).await;
}

struct DenyWrites {
    path: PathBuf,
    sid: String,
    active: bool,
}
impl DenyWrites {
    fn new(path: PathBuf, owned_root: &std::path::Path) -> Self {
        assert!(path.starts_with(owned_root));
        let output = Process::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[Security.Principal.WindowsIdentity]::GetCurrent().User.Value",
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        let sid = String::from_utf8(output.stdout).unwrap().trim().to_owned();
        assert!(sid.starts_with("S-1-"));
        let output = Process::new("icacls.exe")
            .arg(&path)
            .args(["/deny", &format!("*{sid}:(OI)(CI)(W)")])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Self {
            path,
            sid,
            active: true,
        }
    }
    fn restore(&mut self) {
        let output = Process::new("icacls.exe")
            .arg(&self.path)
            .args(["/remove:d", &format!("*{}", self.sid)])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        self.active = false;
    }
}
impl Drop for DenyWrites {
    fn drop(&mut self) {
        if self.active {
            let result = Process::new("icacls.exe")
                .arg(&self.path)
                .args(["/remove:d", &format!("*{}", self.sid)])
                .output();
            if !result.is_ok_and(|output| output.status.success()) {
                eprintln!("Failed to restore isolated ACL: {}", self.path.display());
            }
        }
    }
}
#[tokio::test]
async fn denied_destination_reports_failure_and_recovers_without_redownloading() {
    let fixture = Fixture::new(2 * 1024 * 1024, 0, false).await;
    let (temp, store) = setup();
    let job = enqueue(&store, &fixture.url);
    let mut denied = DenyWrites::new(job.directory.clone(), temp.path());
    assert!(std::fs::write(job.directory.join("must-not-write"), b"x").is_err());
    let failed = run(&store, job).await;
    denied.restore();
    assert_failure(&failed);
    assert_eq!(failed.downloaded, fixture.data.len() as u64);
    let request_count = fixture.requests.lock().unwrap().len();
    recover(&store, failed, &fixture).await;
    assert_eq!(fixture.requests.lock().unwrap().len(), request_count + 1);
}
