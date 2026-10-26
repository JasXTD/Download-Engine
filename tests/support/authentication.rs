use super::*;
use download_engine::{
    auth::{self, Credentials},
    link_recovery::{Command as LinkCommand, Event as LinkEvent, Outcome},
    model::{Plan, Segment},
};
use uuid::Uuid;
// All tests use synthetic values and freshly generated app-owned credential IDs.
// Presence probes follow the same vault serialization rule as reads and writes.
// The direct four-thread vault regression independently checks OS round trips.
fn vault_present(id: Uuid) -> bool {
    auth::credentials_present(id).unwrap()
}
struct Harness {
    scheduler: Option<Scheduler>,
    store: Store,
    _temp: TempDir,
}
impl Harness {
    fn new() -> Self {
        let (temp, store) = setup();
        store.set_queue_running("main", false).unwrap();
        let scheduler = Some(Scheduler::start(store.clone()).unwrap());
        Self {
            scheduler,
            store,
            _temp: temp,
        }
    }
    fn scheduler(&self) -> &Scheduler {
        self.scheduler.as_ref().unwrap()
    }
    fn job(&self, url: &str) -> Job {
        let mut job = Job::new(
            url,
            Some("protected.bin"),
            self.store.paths.downloads.clone(),
            Uuid::new_v4().to_string(),
        )
        .unwrap();
        job.status = Status::Paused;
        self.store.enqueue(&job).unwrap()
    }
    async fn update(
        &self,
        job: &Job,
        credentials: Option<Credentials>,
        restart: bool,
    ) -> auth::Event {
        let request = Uuid::new_v4();
        self.scheduler()
            .send(Command::Authentication(auth::Update {
                request,
                job: job.id.clone(),
                expected_url: job.url.clone(),
                credentials,
                restart_confirmed: restart,
                resume: false,
            }))
            .unwrap();
        tokio::time::timeout(PROGRESS_LIMIT, async {
            loop {
                while let Ok(event) = self.scheduler().auth_events.try_recv() {
                    if event.request == request {
                        return event;
                    }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "No authentication result for {request}; state: {:?}; scheduler errors: {:?}",
                self.store
                    .get(&job.id)
                    .map(|saved| saved.map(|job| (job.status, job.downloaded))),
                self.scheduler().errors.try_iter().collect::<Vec<_>>()
            )
        })
    }
    async fn save(&self, job: &Job, credentials: Credentials) -> Job {
        let event = self.update(job, Some(credentials), false).await;
        assert!(event.result.is_ok(), "{:?}", event.result);
        assert!(
            event.result.unwrap().is_none(),
            "Credential cleanup was deferred"
        );
        self.store.get(&job.id).unwrap().unwrap()
    }
    async fn finish(&self, job: &Job) -> Job {
        let id = job.id.clone();
        let event = until_stalled(&self.store, async {
            loop {
                while let Ok(event) = self.scheduler().finished.try_recv() {
                    if event.job == id {
                        return event;
                    }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "Scheduler did not finish {id}; state: {:?}; errors: {:?}",
                self.store
                    .get(&id)
                    .map(|saved| saved.map(|job| (job.status, job.downloaded))),
                self.scheduler().errors.try_iter().collect::<Vec<_>>()
            )
        });
        assert_eq!(event.status, Status::Complete);
        self.store.get(&id).unwrap().unwrap()
    }
    async fn link(&self, job: &Job, url: &str) -> Uuid {
        let request = Uuid::new_v4();
        self.scheduler()
            .send(Command::Link(LinkCommand::Check {
                request,
                job: job.id.clone(),
                url: url.into(),
            }))
            .unwrap();
        let event = self.link_event(request).await;
        assert!(
            matches!(
                event,
                LinkEvent::Checked {
                    outcome: Outcome::Resume { .. },
                    ..
                }
            ),
            "{event:?}"
        );
        request
    }
    async fn link_event(&self, request: Uuid) -> LinkEvent {
        tokio::time::timeout(PROGRESS_LIMIT, async {
            loop {
                while let Ok(event) = self.scheduler().link_events.try_recv() {
                    let id = match &event {
                        LinkEvent::Progress { request, .. }
                        | LinkEvent::Checked { request, .. }
                        | LinkEvent::Applied { request, .. }
                        | LinkEvent::Failed { request, .. } => *request,
                    };
                    if id == request && !matches!(event, LinkEvent::Progress { .. }) {
                        return event;
                    }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap()
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        drop(self.scheduler.take());
        if let Ok(jobs) = self.store.list() {
            for job in jobs {
                let _ = self.store.remove(&job.id);
            }
        }
    }
}
fn basic() -> Credentials {
    Credentials::basic("test-user".into(), "test-pass".into())
}
fn bearer() -> Credentials {
    Credentials::bearer("test-token_123".into())
}
fn read_stored_files(store: &Store) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&store.paths.data).unwrap() {
        let entry = entry.unwrap();
        // SQLite's shared-memory WAL index contains no database content, and its
        // mandatory Windows byte-range locks prohibit raw reads during a write.
        // See https://sqlite.org/walformat.html#the_wal_index_or_shm_file
        // Store keeps WAL alive; still read the database, WAL and every other file.
        if entry.file_name() == "queue.sqlite3-shm" {
            continue;
        }
        if entry.file_type().unwrap().is_file() {
            let path = entry.path();
            let bytes = std::fs::read(&path)
                .unwrap_or_else(|error| panic!("Cannot inspect {}: {error}", path.display()));
            files.push((path, bytes));
        }
    }
    files
}

#[test]
fn credential_storage_scan_covers_database_and_wal_while_a_writer_is_active() {
    let (_temp, store) = setup();
    let mut database = rusqlite::Connection::open(store.paths.data.join("queue.sqlite3")).unwrap();
    database
        .execute_batch(
            "PRAGMA wal_autocheckpoint=0;
             CREATE TABLE credential_scan_probe(value TEXT NOT NULL);
             PRAGMA wal_checkpoint(TRUNCATE);",
        )
        .unwrap();
    let marker = "synthetic-disk-credential-marker";
    database
        .execute("INSERT INTO credential_scan_probe VALUES (?1)", [marker])
        .unwrap();
    // Ordinary app files must also remain in the scan, even with a similar name.
    let extra = store.paths.data.join("other.sqlite3-shm");
    std::fs::write(&extra, marker).unwrap();
    let writer = database
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let files = read_stored_files(&store);
    let contains_marker = |name: &str, files: &[(std::path::PathBuf, Vec<u8>)]| {
        files
            .iter()
            .find(|(path, _)| path == &store.paths.data.join(name))
            .unwrap_or_else(|| panic!("Storage scan missed {name}"))
            .1
            .windows(marker.len())
            .any(|bytes| bytes == marker.as_bytes())
    };
    assert!(!contains_marker("queue.sqlite3", &files));
    assert!(contains_marker("queue.sqlite3-wal", &files));
    assert!(contains_marker("other.sqlite3-shm", &files));
    writer.rollback().unwrap();
    database
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    assert!(contains_marker("queue.sqlite3", &read_stored_files(&store)));
}

fn seed_parts(store: &Store, mut job: Job, data: &[u8]) -> Job {
    let root = store.paths.parts(&job.id).unwrap();
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("segment-0.part"), &data[..4096]).unwrap();
    job.plan = Some(Plan {
        size: data.len() as u64,
        etag: "\"version-1\"".into(),
        final_url: job.url.clone(),
        segments: vec![Segment {
            start: 0,
            end: data.len() as u64 - 1,
        }],
    });
    job.downloaded = 4096;
    job.total = Some(data.len() as u64);
    store.save(&job).unwrap();
    job
}

#[test]
fn credentials_validate_without_echoing_secrets_and_debug_is_redacted() {
    for credentials in [
        Credentials::basic("name:secret".into(), "private-password".into()),
        Credentials::basic("user".into(), "private\r\npassword".into()),
        Credentials::bearer("Bearer private-token".into()),
        Credentials::bearer("private token".into()),
        Credentials::bearer("=".into()),
    ] {
        let error = credentials.validate().unwrap_err().to_string();
        assert!(!error.contains("private"));
        assert_eq!(format!("{credentials:?}"), "Credentials([redacted])");
    }
    for credentials in [
        basic(),
        bearer(),
        Credentials::bearer("abc+/~._-==".into()),
        Credentials::basic("user".into(), "".into()),
    ] {
        credentials.validate().unwrap();
    }
}

#[tokio::test]
async fn basic_and_bearer_downloads_cover_ranges_redirects_and_completion_cleanup() {
    for mode in [11, 12, 13, 14] {
        let fixture = Fixture::new(8 * 1024 * 1024, mode, false).await;
        let h = Harness::new();
        let job = h.job(&fixture.url.replace("fixture.bin", "redirect"));
        let secured = h
            .save(&job, if mode == 11 { basic() } else { bearer() })
            .await;
        let binding = secured.authentication.clone().unwrap();
        assert!(vault_present(binding.id));
        assert_eq!(binding.origin, auth::origin(&fixture.url).unwrap());
        let json = serde_json::to_string(&secured).unwrap();
        let stored_files = read_stored_files(&h.store);
        for secret in ["test-user", "test-pass", "test-token_123"] {
            assert!(!json.contains(secret));
            for (path, bytes) in &stored_files {
                assert!(
                    !bytes
                        .windows(secret.len())
                        .any(|part| part == secret.as_bytes()),
                    "Credential persisted in {}",
                    path.display()
                );
            }
        }
        let finished = run(&h.store, secured).await;
        assert_eq!(finished.status, Status::Complete, "{:?}", finished.error);
        assert!(finished.authentication.is_none());
        assert!(!vault_present(binding.id));
        assert_eq!(
            std::fs::read(finished.saved_path.unwrap()).unwrap(),
            *fixture.data
        );
        let requests = fixture.requests.lock().unwrap();
        assert!(
            requests.len()
                >= match mode {
                    13 => 2,
                    14 => 4,
                    _ => 6,
                },
            "Expected authenticated redirects and range or single-stream requests"
        );
        assert!(
            requests
                .iter()
                .all(|r| r.to_lowercase().contains("authorization: "))
        );
    }
}

#[tokio::test]
async fn credentials_survive_pause_reopen_and_resume_but_are_not_shared_with_another_job() {
    let fixture = Fixture::pausable_range(2 * 1024 * 1024, 11).await;
    let mut h = Harness::new();
    let secured = h.save(&h.job(&fixture.url), basic()).await;
    let id = secured.authentication.as_ref().unwrap().id;
    let anonymous = run(&h.store, h.job(&fixture.url)).await;
    assert_eq!(anonymous.status, Status::Failed);
    assert!(anonymous.error.unwrap().contains("401"));
    let paused = pause_midway(&h.store, secured, &fixture).await;
    assert!(vault_present(id));
    drop(h.scheduler.take());
    let reopened = Store::open(h.store.paths.clone()).unwrap();
    h.scheduler = Some(Scheduler::start(reopened.clone()).unwrap());
    let resumed = reopened.get(&paused.id).unwrap().unwrap();
    assert_eq!(resumed.authentication.as_ref().unwrap().id, id);
    let finished = run(&reopened, resumed).await;
    assert_eq!(finished.status, Status::Complete, "{:?}", finished.error);
    assert_eq!(
        std::fs::read(finished.saved_path.unwrap()).unwrap(),
        *fixture.data
    );
    assert!(!vault_present(id));
}

#[tokio::test]
async fn failed_or_missing_credentials_leave_a_retryable_job_and_do_not_leak_secrets() {
    let fixture = Fixture::new(64 * 1024, 11, false).await;
    let h = Harness::new();
    let wrong = h
        .save(
            &h.job(&fixture.url),
            Credentials::basic("wrong-user".into(), "never-log-this".into()),
        )
        .await;
    let binding = wrong.authentication.clone().unwrap();
    let failed = run(&h.store, wrong).await;
    assert_eq!(failed.status, Status::Failed);
    let error = failed.error.as_ref().unwrap();
    assert!(error.contains("Authentication") && error.contains("401"));
    assert!(!error.contains("never-log-this") && !error.contains("wrong-user"));
    assert!(vault_present(binding.id));
    // Remove through the same serialized vault API, then restore the stale
    // reference to emulate a queue copied without its Windows credentials.
    assert!(h.update(&failed, None, false).await.result.is_ok());
    assert!(!vault_present(binding.id));
    h.store.save(&failed).unwrap();
    let before = fixture.requests.lock().unwrap().len();
    let missing = run(&h.store, failed).await;
    assert_eq!(fixture.requests.lock().unwrap().len(), before);
    assert!(missing.error.as_ref().unwrap().contains("unavailable"));
    // An empty cache made by the failed probe does not require discarding data.
    assert!(!auth::needs_restart(&h.store, &missing));
    let restored = h.save(&missing, basic()).await;
    assert_eq!(run(&h.store, restored).await.status, Status::Complete);
}

#[tokio::test]
async fn redirects_to_another_port_are_blocked_before_any_request_reaches_it() {
    let destination = Fixture::new(64 * 1024, 0, false).await;
    let fixture = Fixture::new(64 * 1024, 11, false).await;
    let port = reqwest::Url::parse(&destination.url)
        .unwrap()
        .port()
        .unwrap();
    let h = Harness::new();
    let url = fixture
        .url
        .replace("fixture.bin", &format!("cross-origin/{port}"));
    let secured = h.save(&h.job(&url), basic()).await;
    let failed = run(&h.store, secured).await;
    assert_eq!(failed.status, Status::Failed);
    assert!(
        failed.error.as_ref().unwrap().contains("redirect"),
        "{:?}",
        failed.error
    );
    assert!(destination.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn altered_url_or_binding_cannot_send_credentials_to_a_different_server() {
    let destination = Fixture::new(64 * 1024, 0, false).await;
    let fixture = Fixture::new(64 * 1024, 11, false).await;
    let h = Harness::new();
    let mut job = h.save(&h.job(&fixture.url), basic()).await;
    let binding = job.authentication.clone().unwrap();
    job.url = destination.url.clone();
    h.store.save(&job).unwrap();
    let failed = run(&h.store, job.clone()).await;
    assert_eq!(failed.status, Status::Failed);
    assert!(destination.requests.lock().unwrap().is_empty());
    job.authentication.as_mut().unwrap().origin = auth::origin(&destination.url).unwrap();
    h.store.save(&job).unwrap();
    assert_eq!(run(&h.store, job).await.status, Status::Failed);
    assert!(destination.requests.lock().unwrap().is_empty());
    assert!(fixture.requests.lock().unwrap().is_empty());
    assert!(vault_present(binding.id));
}

#[tokio::test]
async fn credential_changes_require_restart_and_remove_obsolete_credentials() {
    let fixture = Fixture::new(128 * 1024, 11, false).await;
    let h = Harness::new();
    let secured = h.save(&h.job(&fixture.url), basic()).await;
    let old = secured.authentication.as_ref().unwrap().id;
    let job = seed_parts(&h.store, secured, &fixture.data);
    let before =
        std::fs::read(h.store.paths.parts(&job.id).unwrap().join("segment-0.part")).unwrap();
    assert!(
        h.update(&job, Some(bearer()), false)
            .await
            .result
            .unwrap_err()
            .contains("Confirm")
    );
    assert_eq!(
        h.store
            .get(&job.id)
            .unwrap()
            .unwrap()
            .authentication
            .as_ref()
            .unwrap()
            .id,
        old
    );
    assert_eq!(
        std::fs::read(h.store.paths.parts(&job.id).unwrap().join("segment-0.part"),).unwrap(),
        before
    );
    assert!(h.update(&job, Some(bearer()), true).await.result.is_ok());
    let updated = h.store.get(&job.id).unwrap().unwrap();
    assert!(updated.plan.is_none() && updated.total.is_none());
    assert_eq!(updated.downloaded, 0);
    assert!(
        !h.store
            .paths
            .parts(&job.id)
            .unwrap()
            .join("segment-0.part")
            .exists()
    );
    assert!(!vault_present(old));
    let fresh = updated.authentication.as_ref().unwrap().id;
    assert!(vault_present(fresh));
    assert!(h.update(&updated, None, false).await.result.is_ok());
    assert!(!vault_present(fresh));
    assert!(
        h.store
            .get(&job.id)
            .unwrap()
            .unwrap()
            .authentication
            .is_none()
    );
}

#[tokio::test]
async fn same_server_link_recovery_reuses_credentials_and_new_servers_never_receive_them() {
    for different_server in [false, true] {
        let fixture = Fixture::new(128 * 1024, 11, false).await;
        let public = Fixture::new(128 * 1024, 0, false).await;
        let h = Harness::new();
        let secured = h.save(&h.job(&fixture.url), basic()).await;
        let binding = secured.authentication.as_ref().unwrap().id;
        let job = seed_parts(&h.store, secured, &fixture.data);
        let url = if different_server {
            public.url.clone()
        } else {
            fixture.url.replace("fixture.bin", "fresh.bin")
        };
        let request = h.link(&job, &url).await;
        // Credential edits and recovery must not own the same job concurrently.
        assert!(
            h.update(&job, Some(basic()), true)
                .await
                .result
                .unwrap_err()
                .contains("link check")
        );
        h.scheduler()
            .send(Command::Link(LinkCommand::Apply {
                request,
                restart: false,
            }))
            .unwrap();
        assert!(matches!(
            h.link_event(request).await,
            LinkEvent::Applied { .. }
        ));
        let updated = h.store.get(&job.id).unwrap().unwrap();
        assert_eq!(updated.authentication.is_some(), !different_server);
        assert_eq!(vault_present(binding), !different_server);
        assert_eq!(h.finish(&updated).await.status, Status::Complete);
        assert!(
            public
                .requests
                .lock()
                .unwrap()
                .iter()
                .all(|r| !r.to_lowercase().contains("authorization:"))
        );
        assert!(!vault_present(binding));
    }
}

#[tokio::test]
async fn insecure_remote_http_stale_urls_and_native_injection_are_rejected() {
    let h = Harness::new();
    let job = h.job("http://example.com/protected.bin");
    assert!(
        h.update(&job, Some(basic()), false)
            .await
            .result
            .unwrap_err()
            .contains("HTTPS")
    );
    assert!(
        h.store
            .get(&job.id)
            .unwrap()
            .unwrap()
            .authentication
            .is_none()
    );
    let mut stale = job.clone();
    stale.url = "https://example.com/changed.bin".into();
    assert!(
        h.update(&stale, Some(basic()), false)
            .await
            .result
            .unwrap_err()
            .contains("link changed")
    );
    for field in ["authentication", "credentials", "authorization"] {
        let body = serde_json::json!({"action":"add","request_id":Uuid::new_v4().to_string(),"url":"https://example.com/file",field:"do-not-accept"});
        let reply = native::handle(&h.store, &serde_json::to_vec(&body).unwrap());
        assert!(!reply.ok);
        assert_eq!(h.store.list().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn removing_jobs_cleans_only_their_own_vault_entries() {
    let h = Harness::new();
    let first = h.save(&h.job("https://example.com/a"), basic()).await;
    let second = h.save(&h.job("https://example.com/b"), bearer()).await;
    let a = first.authentication.unwrap().id;
    let b = second.authentication.unwrap().id;
    h.scheduler().send(Command::Remove(first.id)).unwrap();
    tokio::time::timeout(PROGRESS_LIMIT, async {
        while vault_present(a) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(vault_present(b));
    assert_eq!(
        h.store
            .get(&second.id)
            .unwrap()
            .unwrap()
            .authentication
            .unwrap()
            .id,
        b
    );
    h.store.remove(&second.id).unwrap();
    assert!(h.store.get(&second.id).unwrap().is_none());
    let pending: Vec<String> = rusqlite::Connection::open(h.store.paths.data.join("queue.sqlite3"))
        .unwrap()
        .prepare("SELECT id FROM credential_cleanup")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert!(
        !vault_present(b),
        "Removed credential {b} remains; pending cleanup: {pending:?}"
    );
}

#[tokio::test]
async fn startup_retries_interrupted_cleanup_without_deleting_live_credentials() {
    let mut h = Harness::new();
    let mut job = h.save(&h.job("https://example.com/file"), basic()).await;
    let binding = job.authentication.as_ref().unwrap().id;
    drop(h.scheduler.take());
    let conn = rusqlite::Connection::open(h.store.paths.data.join("queue.sqlite3")).unwrap();
    conn.execute(
        "INSERT INTO credential_cleanup(id) VALUES (?1)",
        [binding.to_string()],
    )
    .unwrap();
    h.scheduler = Some(Scheduler::start(h.store.clone()).unwrap());
    assert!(
        h.scheduler()
            .errors
            .try_iter()
            .any(|e| e.contains("live download"))
    );
    assert!(vault_present(binding));
    drop(h.scheduler.take());
    job.authentication = None;
    h.store.save(&job).unwrap();
    h.scheduler = Some(Scheduler::start(h.store.clone()).unwrap());
    assert!(!vault_present(binding));
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM credential_cleanup", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn authentication_reset_refuses_cache_junctions_and_rolls_back_vault_staging() {
    use std::os::windows::process::CommandExt;
    let h = Harness::new();
    let job = h.job("https://example.com/protected");
    let foreign = h._temp.path().join("foreign");
    std::fs::create_dir(&foreign).unwrap();
    let preserved = foreign.join("whole.part");
    std::fs::write(&preserved, b"unrelated data").unwrap();
    let windows = download_engine::paths::file_browser()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let result = std::process::Command::new(windows.join("System32").join("cmd.exe"))
        .args(["/d", "/c", "mklink", "/J"])
        .arg(h.store.paths.parts(&job.id).unwrap())
        .arg(&foreign)
        .creation_flags(0x08000000)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "Cannot create isolated junction: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result = h.update(&job, Some(basic()), true).await.result;
    assert!(result.unwrap_err().contains("linked download cache"));
    assert_eq!(std::fs::read(preserved).unwrap(), b"unrelated data");
    assert!(
        h.store
            .get(&job.id)
            .unwrap()
            .unwrap()
            .authentication
            .is_none()
    );
    let conn = rusqlite::Connection::open(h.store.paths.data.join("queue.sqlite3")).unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM credential_cleanup", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn active_downloads_reject_credential_changes_until_pause_finishes() {
    // Hold the transfer open: under load the update can wait longer than any
    // paced transfer takes, and a finished job reports a different error.
    let fixture = Fixture::pausable_range(2 * 1024 * 1024, 11).await;
    let h = Harness::new();
    let job = h.save(&h.job(&fixture.url), basic()).await;
    let binding = job.authentication.as_ref().unwrap().id;
    h.scheduler()
        .send(Command::StartQueue("main".into()))
        .unwrap();
    h.scheduler().send(Command::Resume(job.id.clone())).unwrap();
    wait_until(&h.store, || fixture.requests.lock().unwrap().len() >= 2).await;
    let event = h.update(&job, Some(bearer()), true).await;
    assert!(event.result.unwrap_err().contains("pause"));
    assert_eq!(
        h.store
            .get(&job.id)
            .unwrap()
            .unwrap()
            .authentication
            .unwrap()
            .id,
        binding
    );
    h.scheduler().send(Command::Pause(job.id.clone())).unwrap();
    wait_until(&h.store, || {
        h.store.get(&job.id).unwrap().unwrap().status == Status::Paused
    })
    .await;
    // The held response would end in a network timeout and a retry instead.
    let paused = h.store.get(&job.id).unwrap().unwrap();
    assert!(paused.retry.is_default(), "{:?}", paused.retry);
    assert!(h.update(&job, Some(bearer()), true).await.result.is_ok());
    assert!(!vault_present(binding));
}

#[tokio::test]
async fn cleanup_failure_keeps_completion_durable_and_still_removes_partial_files() {
    let fixture = Fixture::new(64 * 1024, 11, false).await;
    let mut h = Harness::new();
    let finished = h.save(&h.job(&fixture.url), basic()).await;
    let retired = finished.authentication.as_ref().unwrap().id;
    let blocker = h.save(&h.job(&fixture.url), basic()).await;
    let live = blocker.authentication.unwrap().id;
    let conn = rusqlite::Connection::open(h.store.paths.data.join("queue.sqlite3")).unwrap();
    conn.execute(
        "INSERT INTO credential_cleanup(id) VALUES(?1)",
        [live.to_string()],
    )
    .unwrap();
    let result = engine::run(
        h.store.clone(),
        finished.clone(),
        CancellationToken::new(),
        download_engine::bandwidth::Limiter::default(),
    )
    .await;
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Download completed")
    );
    let saved = h.store.get(&finished.id).unwrap().unwrap();
    assert_eq!(saved.status, Status::Complete);
    assert!(saved.authentication.is_none());
    assert_eq!(
        std::fs::read(saved.saved_path.unwrap()).unwrap(),
        *fixture.data
    );
    assert!(!h.store.paths.parts(&saved.id).unwrap().exists());
    assert!(vault_present(live));
    conn.execute(
        "DELETE FROM credential_cleanup WHERE id=?1",
        [live.to_string()],
    )
    .unwrap();
    drop(h.scheduler.take());
    h.scheduler = Some(Scheduler::start(h.store.clone()).unwrap());
    assert!(!vault_present(retired));
    assert!(vault_present(live));
}

#[tokio::test]
async fn committed_replacement_link_reports_cleanup_warning_as_success() {
    let fixture = Fixture::new(64 * 1024, 11, false).await;
    let public = Fixture::new(64 * 1024, 0, false).await;
    let h = Harness::new();
    let secured = h.save(&h.job(&fixture.url), basic()).await;
    let job = seed_parts(&h.store, secured, &fixture.data);
    let blocker = h.save(&h.job(&fixture.url), basic()).await;
    let live = blocker.authentication.unwrap().id;
    let request = h.link(&job, &public.url).await;
    let conn = rusqlite::Connection::open(h.store.paths.data.join("queue.sqlite3")).unwrap();
    conn.execute(
        "INSERT INTO credential_cleanup(id) VALUES(?1)",
        [live.to_string()],
    )
    .unwrap();
    h.scheduler()
        .send(Command::Link(LinkCommand::Apply {
            request,
            restart: false,
        }))
        .unwrap();
    assert!(matches!(
        h.link_event(request).await,
        LinkEvent::Applied {
            warning: Some(_),
            ..
        }
    ));
    let updated = h.store.get(&job.id).unwrap().unwrap();
    assert_eq!(updated.url, public.url);
    assert!(updated.authentication.is_none());
    assert_eq!(updated.status, Status::Queued);
    assert!(vault_present(live));
    conn.execute(
        "DELETE FROM credential_cleanup WHERE id=?1",
        [live.to_string()],
    )
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authenticated_retry_keeps_scoped_credentials_until_completion() {
    let fixture = Fixture::new(300000, 7, false).await;
    let h = Harness::new();
    let mut job = h.job(&fixture.url);
    job.retry = download_engine::retry::State {
        attempts: 3,
        next_at: None,
    };
    h.store.save(&job).unwrap();
    let job = h
        .save(
            &job,
            Credentials::basic("test-user".into(), "test-pass".into()),
        )
        .await;
    assert!(
        job.retry.is_default(),
        "Changed authentication starts a fresh budget"
    );
    let binding = job.authentication.as_ref().unwrap().id;
    h.scheduler().send(Command::Resume(job.id.clone())).unwrap();
    h.scheduler()
        .send(Command::StartQueue("main".into()))
        .unwrap();
    wait_until(&h.store, || {
        h.store
            .get(&job.id)
            .unwrap()
            .unwrap()
            .retry
            .next_at
            .is_some()
    })
    .await;
    let pending = h.store.get(&job.id).unwrap().unwrap();
    assert_eq!(pending.retry.attempts, 1);
    assert_eq!(pending.authentication.as_ref().unwrap().id, binding);
    assert!(vault_present(binding));
    fixture.mode.store(11, Ordering::SeqCst);
    wait_until(&h.store, || {
        h.store.get(&job.id).unwrap().unwrap().status == Status::Complete
    })
    .await;
    let done = h.store.get(&job.id).unwrap().unwrap();
    assert!(done.authentication.is_none());
    assert!(done.retry.is_default());
    assert_eq!(
        std::fs::read(done.saved_path.unwrap()).unwrap(),
        *fixture.data
    );
    wait_until(&h.store, || !vault_present(binding)).await;
    assert!(fixture.requests.lock().unwrap().iter().all(|r| {
        r.to_ascii_lowercase().contains(
            "authorization: basic dgvzdc11c2vyOnRlc3QtcGFzcw=="
                .to_ascii_lowercase()
                .as_str(),
        )
    }));
}
