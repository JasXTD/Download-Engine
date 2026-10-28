use super::*;
use download_engine::{
    capture::ExpectedFile,
    link_recovery::{Command as LinkCommand, Event, Outcome},
    model::{Plan, Segment},
};
use uuid::Uuid;

fn paused(store: &Store, data: &[u8]) -> Job {
    let mut job = Job::new(
        "http://127.0.0.1:1/expired.bin?token=old-secret",
        Some("kept-name.bin"),
        store.paths.downloads.clone(),
        Uuid::new_v4().to_string(),
    )
    .unwrap();
    job.status = Status::Failed;
    job.retry = download_engine::retry::State {
        attempts: 3,
        next_at: None,
    };
    job.error = Some("Server returned HTTP 403".into());
    let size = data.len() as u64;
    job.total = Some(size);
    let step = size / 4;
    job.plan = Some(Plan {
        size,
        etag: "\"version-1\"".into(),
        final_url: job.url.clone(),
        segments: (0..4)
            .map(|n| Segment {
                start: n * step,
                end: (n + 1) * step - 1,
            })
            .collect(),
    });
    job.expected_file = Some(ExpectedFile {
        size,
        mime: "application/octet-stream".into(),
        etag: Some("\"version-1\"".into()),
    });
    let root = store.paths.parts(&job.id).unwrap();
    std::fs::create_dir(&root).unwrap();
    for (n, part) in job.plan.as_ref().unwrap().segments.iter().enumerate() {
        let len = (step / 3) as usize + n;
        let start = part.start as usize;
        std::fs::write(
            root.join(format!("segment-{n}.part")),
            &data[start..start + len],
        )
        .unwrap();
        job.downloaded += len as u64;
    }
    let queue = store.create_queue(&format!("Later-{}", job.id)).unwrap();
    store.enqueue_batch(&[job.clone()], &queue.id).unwrap();
    job
}
fn snapshot(store: &Store, job: &Job) -> (serde_json::Value, Vec<Vec<u8>>) {
    (
        serde_json::to_value(store.get(&job.id).unwrap().unwrap()).unwrap(),
        (0..4)
            .map(|n| {
                std::fs::read(
                    store
                        .paths
                        .parts(&job.id)
                        .unwrap()
                        .join(format!("segment-{n}.part")),
                )
                .unwrap()
            })
            .collect(),
    )
}
async fn event(scheduler: &Scheduler, request: Uuid) -> Event {
    tokio::time::timeout(PROGRESS_LIMIT, async {
        loop {
            while let Ok(event) = scheduler.link_events.try_recv() {
                let id = match &event {
                    Event::Progress { request, .. }
                    | Event::Checked { request, .. }
                    | Event::Applied { request, .. }
                    | Event::Failed { request, .. } => *request,
                };
                if id == request && !matches!(event, Event::Progress { .. }) {
                    return event;
                }
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("No terminal link event")
}
fn begin(scheduler: &Scheduler, job: &Job, url: &str) -> Uuid {
    let request = Uuid::new_v4();
    scheduler
        .send(Command::Link(LinkCommand::Check {
            request,
            job: job.id.clone(),
            url: url.into(),
        }))
        .unwrap();
    request
}
async fn checked(scheduler: &Scheduler, job: &Job, url: &str) -> (Uuid, Outcome) {
    let request = begin(scheduler, job, url);
    let Event::Checked { outcome, .. } = event(scheduler, request).await else {
        panic!("Link check failed");
    };
    (request, outcome)
}
async fn apply(scheduler: &Scheduler, request: Uuid, restart: bool) -> Event {
    scheduler
        .send(Command::Link(LinkCommand::Apply { request, restart }))
        .unwrap();
    event(scheduler, request).await
}

fn browser_offer(store: &Store, job: &Job, url: &str) -> download_engine::browser_recovery::Lease {
    let lease = store
        .begin_browser_recovery(&job.id, "https://original.test/download-page")
        .unwrap();
    // Exercise the same typed protocol used by the browser host.
    let request = serde_json::from_value(
        serde_json::json!({"action":"recovery_offer","id":lease.id,"file":{
            "url":url,"filename":job.name,"size":job.total,"mime":"application/octet-stream"
        }}),
    )
    .unwrap();
    let reply = download_engine::native::handle_request(store, request);
    assert!(reply.ok, "{}", reply.message);
    assert_eq!(reply.recovery.unwrap().state, "offered");
    lease
}
async fn browser_check(scheduler: &Scheduler, recovery: Uuid) -> (Uuid, Outcome) {
    let request = Uuid::new_v4();
    scheduler
        .send(Command::Link(LinkCommand::CheckBrowser {
            request,
            recovery,
        }))
        .unwrap();
    let Event::Checked { outcome, .. } = event(scheduler, request).await else {
        panic!("Browser link check failed")
    };
    (request, outcome)
}
#[tokio::test]
async fn browser_recovery_commits_receipt_and_existing_job_together_then_resumes_exact_bytes() {
    let fixture = Fixture::new(1024 * 1024, 2, false).await;
    let (_temp, store) = setup();
    let original = paused(&store, &fixture.data);
    let before = snapshot(&store, &original);
    let queue = store.job_queue(&original.id).unwrap();
    let scheduler = Scheduler::start(store.clone()).unwrap();
    let lease = browser_offer(&store, &original, &fixture.url);
    let (request, outcome) = browser_check(&scheduler, lease.id).await;
    assert_eq!(
        outcome,
        Outcome::Resume {
            saved: original.downloaded,
            total: fixture.data.len() as u64
        }
    );
    assert_eq!(snapshot(&store, &original), before);
    assert_eq!(lease.refresh().unwrap().state, "offered");
    assert!(matches!(
        apply(&scheduler, request, false).await,
        Event::Applied { .. }
    ));
    assert_eq!(lease.refresh().unwrap().state, "accepted");
    let id = lease.id;
    drop(lease);
    assert_eq!(
        store.browser_recovery_status(id).unwrap().unwrap().state,
        "accepted"
    );
    let updated = store.get(&original.id).unwrap().unwrap();
    assert_eq!(
        store.list().unwrap().len(),
        1,
        "recovery must never enqueue a duplicate"
    );
    assert_eq!(store.job_queue(&updated.id).unwrap(), queue);
    assert_eq!(
        (&updated.name, &updated.directory, &updated.request_id),
        (&original.name, &original.directory, &original.request_id)
    );
    assert_eq!(
        updated.source_page.as_deref(),
        Some("https://original.test/download-page")
    );
    drop(scheduler);
    let finished = run(&store, updated).await;
    assert_eq!(finished.status, Status::Complete, "{:?}", finished.error);
    assert_eq!(
        std::fs::read(finished.saved_path.unwrap()).unwrap(),
        *fixture.data
    );
}
#[tokio::test]
async fn browser_cancellation_and_failed_commit_never_acknowledge_or_change_saved_data() {
    for cancel in [false, true] {
        let fixture = Fixture::new(256 * 1024, 2, false).await;
        let (_temp, store) = setup();
        let original = paused(&store, &fixture.data);
        let before = snapshot(&store, &original);
        let scheduler = Scheduler::start(store.clone()).unwrap();
        let lease = browser_offer(&store, &original, &fixture.url);
        let (request, _) = browser_check(&scheduler, lease.id).await;
        if cancel {
            store.cancel_browser_recovery(lease.id).unwrap();
        } else {
            let db = crate::fixture_database::open(store.paths.data.join("queue.sqlite3")).unwrap();
            db.execute_batch("CREATE TRIGGER refuse_recovery BEFORE UPDATE ON jobs BEGIN SELECT RAISE(FAIL,'injected write failure'); END;").unwrap();
        }
        assert!(matches!(
            apply(&scheduler, request, false).await,
            Event::Failed { .. }
        ));
        assert_eq!(snapshot(&store, &original), before);
        assert_ne!(
            store
                .browser_recovery_status(lease.id)
                .unwrap()
                .unwrap()
                .state,
            "accepted"
        );
    }
}
#[tokio::test]
async fn browser_wrong_content_requires_explicit_restart_and_preserves_partial_data() {
    let fixture = Fixture::new(256 * 1024, 9, false).await;
    let (_temp, store) = setup();
    let original = paused(&store, &fixture.data);
    let before = snapshot(&store, &original);
    let scheduler = Scheduler::start(store.clone()).unwrap();
    let lease = browser_offer(&store, &original, &fixture.url);
    let (request, outcome) = browser_check(&scheduler, lease.id).await;
    assert!(matches!(outcome, Outcome::RestartRequired(_)));
    assert!(matches!(
        apply(&scheduler, request, false).await,
        Event::Failed { .. }
    ));
    assert_eq!(snapshot(&store, &original), before);
    assert_eq!(lease.refresh().unwrap().state, "offered");
}

// Explicit integration run only: Chrome and Edge get disposable profiles, and
// the helper removes its unique test-only native-host registration in its finally block.
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires installed Chrome/Edge, cached Playwright and an explicit isolated browser run"]
async fn live_chrome_edge_browser_recovery() {
    use sha2::Digest;
    let workspace = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let artifact = workspace
        .join("artifacts")
        .join(format!("browser-recovery-{}", Uuid::new_v4()));
    let host = std::env::var_os("IDM_EX_TEST_HOST")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| workspace.join("target/debug/de-host.exe"));
    let node = std::env::var_os("IDM_EX_TEST_NODE")
        .expect("Set IDM_EX_TEST_NODE to the installed Node binary");
    let extension = std::env::var_os("IDM_EX_TEST_EXTENSION")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| workspace.join("extension"));
    for browser in ["chrome", "edge"] {
        let root = artifact.join(browser);
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(AppPaths {
            data: root.join("data"),
            downloads: root.join("downloads"),
        })
        .unwrap();
        let fixture = Fixture::new(4 * 1024 * 1024, 2, true).await;
        fixture
            .disposition
            .lock()
            .unwrap()
            .push("attachment; filename=kept-name.bin".into());
        let jobs = [paused(&store, &fixture.data), paused(&store, &fixture.data)];
        let original = snapshot(&store, &jobs[0]);
        let page = fixture.url.replace("fixture.bin", "page");
        let config = serde_json::json!({"browser":browser,"run":root,"host":host,"extension":extension,"url":fixture.url,"page":page,
            "jobs":jobs.iter().map(|j|&j.id).collect::<Vec<_>>(),"sha256":format!("{:x}",sha2::Sha256::digest(&*fixture.data))});
        let config_path = root.join("config.json");
        std::fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
        let scheduler = Scheduler::start(store.clone()).unwrap();
        let log = std::fs::File::create(root.join("browser.log")).unwrap();
        let mut child = std::process::Command::new(&node)
            .arg(workspace.join("scripts/browser_recovery_smoke.cjs"))
            .arg(&config_path)
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap();
        for (index, job) in jobs.iter().enumerate() {
            let lease = store.begin_browser_recovery(&job.id, &page).unwrap();
            let mut offered = false;
            // The browser helper has bounded waits and removes its test-only
            // registration; allow it to report a failure before returning here.
            loop {
                if child.try_wait().unwrap().is_some() {
                    break;
                }
                let session = lease.refresh().unwrap();
                if session.state == "offered" && root.join(format!("paused-{index}")).exists() {
                    offered = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(75)).await;
            }
            if !offered {
                break;
            }
            if index == 0 {
                store.cancel_browser_recovery(lease.id).unwrap();
                assert_eq!(snapshot(&store, job), original);
            } else {
                let (request, outcome) = browser_check(&scheduler, lease.id).await;
                assert!(matches!(outcome, Outcome::Resume { .. }));
                assert!(matches!(
                    apply(&scheduler, request, false).await,
                    Event::Applied { .. }
                ));
                let queue = store.job_queue(&job.id).unwrap().unwrap();
                store.set_queue_running(&queue, true).unwrap();
            }
        }
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(75)).await;
        };
        assert!(
            status.success(),
            "{browser} failed: {}",
            std::fs::read_to_string(root.join("browser.log")).unwrap()
        );
        assert_eq!(snapshot(&store, &jobs[0]), original);
        println!(
            "Browser verification: {}",
            root.join("verification.json").display()
        );
    }
}

#[tokio::test]
async fn new_url_and_validator_resume_every_saved_segment_and_preserve_queue_and_identity() {
    let fixture = Fixture::new(1024 * 1024, 2, false).await;
    let (_temp, store) = setup();
    let original = paused(&store, &fixture.data);
    let before = snapshot(&store, &original);
    let queue = store.job_queue(&original.id).unwrap().unwrap();
    let another = Job::new(
        &fixture.url,
        Some("another.bin"),
        store.paths.downloads.clone(),
        Uuid::new_v4().to_string(),
    )
    .unwrap();
    store.enqueue_batch(&[another], &queue).unwrap();
    let order: Vec<_> = store
        .queue_jobs(&queue)
        .unwrap()
        .iter()
        .map(|j| j.id.clone())
        .collect();
    let scheduler = Scheduler::start(store.clone()).unwrap();
    let redirected = fixture.url.replace("fixture.bin", "redirect");
    let (request, outcome) = checked(&scheduler, &original, &redirected).await;
    assert_eq!(
        outcome,
        Outcome::Resume {
            saved: original.downloaded,
            total: fixture.data.len() as u64
        }
    );
    assert_eq!(
        snapshot(&store, &original),
        before,
        "check must be read-only"
    );
    assert!(matches!(
        apply(&scheduler, request, false).await,
        Event::Applied { .. }
    ));
    let updated = store.get(&original.id).unwrap().unwrap();
    assert_eq!(updated.url, redirected);
    assert_eq!(updated.plan.as_ref().unwrap().etag, "\"version-2\"");
    assert_eq!(updated.plan.as_ref().unwrap().final_url, fixture.url);
    assert!(updated.expected_file.is_none());
    assert_eq!(
        (
            &updated.id,
            &updated.request_id,
            &updated.name,
            &updated.directory,
            updated.created
        ),
        (
            &original.id,
            &original.request_id,
            &original.name,
            &original.directory,
            original.created
        )
    );
    assert_eq!(
        store
            .queue_jobs(&queue)
            .unwrap()
            .iter()
            .map(|j| j.id.clone())
            .collect::<Vec<_>>(),
        order
    );
    assert_eq!(updated.status, Status::Queued);
    assert!(updated.retry.is_default());
    let persisted = Store::open(store.paths.clone())
        .unwrap()
        .get(&updated.id)
        .unwrap()
        .unwrap();
    assert_eq!(persisted.url, redirected);
    drop(scheduler);
    let complete = run(&store, persisted).await;
    assert_eq!(complete.status, Status::Complete, "{:?}", complete.error);
    assert_eq!(
        std::fs::read(complete.saved_path.unwrap()).unwrap(),
        *fixture.data
    );
    let requests = fixture.requests.lock().unwrap();
    for segment in &original.plan.as_ref().unwrap().segments {
        assert!(
            requests
                .iter()
                .any(|r| r.contains(&format!("range: bytes={}-", segment.start))
                    && r.contains("if-range: \"version-2\"")),
            "unchecked segment {}",
            segment.start
        );
    }
}

#[tokio::test]
async fn identical_etag_and_size_at_a_different_url_do_not_allow_mixed_bytes() {
    let fixture = Fixture::new(512 * 1024, 9, false).await;
    let (_temp, store) = setup();
    let original = paused(&store, &fixture.data);
    let before = snapshot(&store, &original);
    let scheduler = Scheduler::start(store.clone()).unwrap();
    let (request, outcome) = checked(&scheduler, &original, &fixture.url).await;
    assert!(
        matches!(outcome, Outcome::RestartRequired(ref reason) if reason.contains("does not match"))
    );
    assert_eq!(snapshot(&store, &original), before);
    assert!(matches!(
        apply(&scheduler, request, false).await,
        Event::Failed { .. }
    ));
    assert_eq!(
        snapshot(&store, &original),
        before,
        "unconfirmed restart must preserve all bytes"
    );
}

#[tokio::test]
async fn unresumable_replacements_require_explicit_restart_and_keep_completed_files() {
    for mode in [1, 5] {
        let fixture = Fixture::new(128 * 1024, mode, false).await;
        let (_temp, store) = setup();
        let original = paused(&store, &fixture.data);
        let before = snapshot(&store, &original);
        let existing = store.paths.downloads.join(&original.name);
        std::fs::write(&existing, b"existing completed file").unwrap();
        let scheduler = Scheduler::start(store.clone()).unwrap();
        let (request, outcome) = checked(&scheduler, &original, &fixture.url).await;
        assert!(matches!(outcome, Outcome::RestartRequired(_)));
        assert_eq!(snapshot(&store, &original), before);
        assert!(matches!(
            apply(&scheduler, request, true).await,
            Event::Applied { .. }
        ));
        assert!(!store.paths.parts(&original.id).unwrap().exists());
        let updated = store.get(&original.id).unwrap().unwrap();
        assert!(updated.plan.is_none());
        assert_eq!(updated.downloaded, 0);
        drop(scheduler);
        let complete = run(&store, updated).await;
        assert_eq!(complete.status, Status::Complete, "{:?}", complete.error);
        assert_eq!(
            std::fs::read(complete.saved_path.unwrap()).unwrap(),
            *fixture.data
        );
        assert_eq!(std::fs::read(existing).unwrap(), b"existing completed file");
    }
}

#[tokio::test]
async fn size_change_and_single_connection_partials_offer_restart() {
    for has_plan in [true, false] {
        let fixture = Fixture::new(128 * 1024, 0, false).await;
        let (_temp, store) = setup();
        let mut original = paused(&store, &vec![5; 256 * 1024]);
        if !has_plan {
            original.plan = None;
            store.save(&original).unwrap();
            std::fs::write(
                store.paths.parts(&original.id).unwrap().join("whole.part"),
                vec![5; 1024],
            )
            .unwrap();
        }
        let scheduler = Scheduler::start(store.clone()).unwrap();
        let (request, outcome) = checked(&scheduler, &original, &fixture.url).await;
        assert!(matches!(outcome, Outcome::RestartRequired(_)));
        scheduler
            .send(Command::Link(LinkCommand::Cancel(request)))
            .unwrap();
        drop(scheduler);
        assert_eq!(store.get(&original.id).unwrap().unwrap().url, original.url);
        if !has_plan {
            assert_eq!(
                std::fs::read(store.paths.parts(&original.id).unwrap().join("whole.part"),)
                    .unwrap(),
                vec![5; 1024]
            );
        }
    }
}

#[tokio::test]
async fn network_protocol_and_login_failures_leave_link_and_partials_untouched() {
    for mode in [3, 4, 6, 7, 8, 10] {
        let fixture = Fixture::new(128 * 1024, mode, false).await;
        let (_temp, store) = setup();
        let original = paused(&store, &fixture.data);
        let before = snapshot(&store, &original);
        let scheduler = Scheduler::start(store.clone()).unwrap();
        let request = begin(
            &scheduler,
            &original,
            &format!("{}?token=never-log-me", fixture.url),
        );
        let result = event(&scheduler, request).await;
        let Event::Failed { message, .. } = result else {
            panic!("mode {mode} accepted: {result:?}");
        };
        assert!(!message.contains("never-log-me"));
        assert!(!message.contains(&fixture.url));
        assert_eq!(snapshot(&store, &original), before);
    }
}

#[tokio::test]
async fn cancelled_check_and_app_shutdown_preserve_all_saved_work() {
    for close_app in [false, true] {
        // Hold the verified bytes, so the check is still running when Cancel or
        // shutdown arrives. Under load either can wait out a long scheduler tick,
        // longer than verifying a paced response takes.
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let fixture = Fixture::with_body_gate(4 * 1024 * 1024, 0, false, Some(gate)).await;
        let (_temp, store) = setup();
        let original = paused(&store, &fixture.data);
        let before = snapshot(&store, &original);
        let scheduler = Scheduler::start(store.clone()).unwrap();
        let request = begin(&scheduler, &original, &fixture.url);
        tokio::time::timeout(PROGRESS_LIMIT, async {
            while fixture.requests.lock().unwrap().len() < 2 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        if !close_app {
            scheduler
                .send(Command::Link(LinkCommand::Cancel(request)))
                .unwrap();
            scheduler
                .send(Command::Link(LinkCommand::Apply {
                    request,
                    restart: false,
                }))
                .unwrap();
            assert!(matches!(
                event(&scheduler, request).await,
                Event::Failed { .. }
            ));
        }
        let start = std::time::Instant::now();
        drop(scheduler);
        // The held response ends only by cancellation or the engine's 30 s
        // network timeout; shutdown waited up to 18 s for a busy tick under load.
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "shutdown did not cancel validation promptly"
        );
        assert_eq!(snapshot(&store, &original), before);
    }
}

#[tokio::test]
async fn check_reserves_the_job_against_resume_restart_remove_and_reordering() {
    let fixture = Fixture::new(256 * 1024, 0, false).await;
    let (_temp, store) = setup();
    let original = paused(&store, &fixture.data);
    let scheduler = Scheduler::start(store.clone()).unwrap();
    let (request, _) = checked(&scheduler, &original, &fixture.url).await;
    let queue = store.job_queue(&original.id).unwrap().unwrap();
    let before = snapshot(&store, &original);
    for command in [
        Command::Resume(original.id.clone()),
        Command::Restart(original.id.clone()),
        Command::Remove(original.id.clone()),
        Command::ReorderJob(original.id.clone(), true),
        Command::MoveJob(original.id.clone(), "main".into()),
    ] {
        scheduler.send(command).unwrap();
    }
    scheduler.send(Command::StartQueue(queue)).unwrap();
    tokio::time::timeout(PROGRESS_LIMIT, async {
        let mut rejected = 0;
        while rejected < 5 {
            rejected += scheduler
                .errors
                .try_iter()
                .filter(|e| e.contains("link check"))
                .count();
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(snapshot(&store, &original), before);
    scheduler
        .send(Command::Link(LinkCommand::Cancel(request)))
        .unwrap();
}

#[tokio::test]
async fn stale_or_removed_job_cannot_be_overwritten_by_a_checked_link() {
    for remove in [false, true] {
        let fixture = Fixture::new(128 * 1024, 0, false).await;
        let (_temp, store) = setup();
        let original = paused(&store, &fixture.data);
        let scheduler = Scheduler::start(store.clone()).unwrap();
        let (request, _) = checked(&scheduler, &original, &fixture.url).await;
        if remove {
            store.remove(&original.id).unwrap();
        } else {
            let mut changed = original.clone();
            changed.name = "edited-name.bin".into();
            store.save(&changed).unwrap();
        }
        assert!(matches!(
            apply(&scheduler, request, false).await,
            Event::Failed { .. }
        ));
        let current = store.get(&original.id).unwrap();
        if remove {
            assert!(current.is_none());
        } else {
            let current = current.unwrap();
            assert_eq!(current.name, "edited-name.bin");
            assert_eq!(current.url, original.url);
        }
    }
}

#[tokio::test]
async fn empty_files_and_no_partial_data_can_start_with_a_new_link() {
    for (size, name) in [
        (0, Some("fresh.bin")),
        (4096, Some("fresh.bin")),
        (4096, None),
    ] {
        let fixture = Fixture::new(size, 0, false).await;
        *fixture.disposition.lock().unwrap() = vec!["attachment; filename=changed.pdf".into()];
        let (_temp, store) = setup();
        let mut job = Job::new(
            "https://example.com/expired",
            name,
            store.paths.downloads.clone(),
            Uuid::new_v4().to_string(),
        )
        .unwrap();
        let expected_name = job.name.clone();
        job.status = Status::Paused;
        let queue = store.create_queue("Later").unwrap();
        store.enqueue_batch(&[job.clone()], &queue.id).unwrap();
        let scheduler = Scheduler::start(store.clone()).unwrap();
        let (request, outcome) = checked(&scheduler, &job, &fixture.url).await;
        assert_eq!(outcome, Outcome::Fresh);
        assert!(matches!(
            apply(&scheduler, request, false).await,
            Event::Applied { .. }
        ));
        drop(scheduler);
        let complete = run(&store, store.get(&job.id).unwrap().unwrap()).await;
        assert_eq!(complete.status, Status::Complete);
        assert_eq!(complete.name, expected_name);
        assert!(!complete.server_name_pending);
        assert_eq!(
            std::fs::read(complete.saved_path.unwrap()).unwrap(),
            *fixture.data
        );
    }
}

#[tokio::test]
async fn invalid_links_active_states_and_pending_publication_are_rejected() {
    let fixture = Fixture::new(128 * 1024, 0, false).await;
    let (_temp, store) = setup();
    let mut original = paused(&store, &fixture.data);
    let scheduler = Scheduler::start(store.clone()).unwrap();
    for url in [
        "file:///C:/private.txt",
        "https://user:secret@example.com/file",
        "not a URL",
    ] {
        let request = begin(&scheduler, &original, url);
        assert!(matches!(
            event(&scheduler, request).await,
            Event::Failed { .. }
        ));
    }
    for status in [Status::Queued, Status::Downloading, Status::Complete] {
        original.status = status;
        store.save(&original).unwrap();
        let request = begin(&scheduler, &original, &fixture.url);
        assert!(matches!(
            event(&scheduler, request).await,
            Event::Failed { .. }
        ));
    }
    original.status = Status::Paused;
    original.saved_path = Some(original.directory.join(&original.name));
    store.save(&original).unwrap();
    let request = begin(&scheduler, &original, &fixture.url);
    assert!(matches!(
        event(&scheduler, request).await,
        Event::Failed { .. }
    ));
    assert!(fixture.requests.lock().unwrap().is_empty());
}

#[cfg(windows)]
#[tokio::test]
async fn linked_partial_files_are_rejected_and_verification_handles_prevent_tampering() {
    let fixture = Fixture::new(128 * 1024, 0, false).await;
    let (_temp, store) = setup();
    let original = paused(&store, &fixture.data);
    let part = store
        .paths
        .parts(&original.id)
        .unwrap()
        .join("segment-0.part");
    let foreign = store.paths.downloads.join("protected.bin");
    std::fs::hard_link(&part, &foreign).unwrap();
    let scheduler = Scheduler::start(store.clone()).unwrap();
    let request = begin(&scheduler, &original, &fixture.url);
    assert!(matches!(
        event(&scheduler, request).await,
        Event::Failed { .. }
    ));
    std::fs::remove_file(&foreign).unwrap();
    let (request, _) = checked(&scheduler, &original, &fixture.url).await;
    assert!(std::fs::OpenOptions::new().write(true).open(&part).is_err());
    assert!(std::fs::remove_file(&part).is_err());
    scheduler
        .send(Command::Link(LinkCommand::Cancel(request)))
        .unwrap();
    drop(scheduler);
    assert!(std::fs::OpenOptions::new().write(true).open(part).is_ok());
}

async fn completed(
    store: &Store,
    id: &str,
    fixture: &Fixture,
    scheduler: &Scheduler,
) -> Result<Job, String> {
    let mut last = None;
    let observed = until_stalled(store, async {
        loop {
            let errors: Vec<_> = scheduler.errors.try_iter().collect();
            if !errors.is_empty() {
                return Err(format!(
                    "Scheduler failed while waiting for completion: {}",
                    errors.join("; ")
                ));
            }
            let job = read_job(store, id).await?;
            let complete = job.status == Status::Complete;
            let failed =
                job.retry.attempts > 0 || matches!(job.status, Status::Failed | Status::Paused);
            last = Some(job);
            // This fixture promises a successful transfer. A retry is an error
            // in this scenario, not a reason to spin until the watchdog expires.
            if failed {
                return Err(format!(
                    "Completion failed; {}",
                    describe_job(last.as_ref().unwrap())
                ));
            }
            if complete {
                return Ok::<_, String>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    let state = last
        .as_ref()
        .map_or_else(|| "no state read".into(), describe_job);
    observed.map_err(|_| {
        format!(
            "Completion stalled; {state}; requests={}; scheduler errors={:?}",
            fixture.requests.lock().unwrap().len(),
            scheduler.errors.try_iter().collect::<Vec<_>>()
        )
    })??;
    Ok(last.unwrap())
}
#[tokio::test]
async fn completion_wait_reports_retry_failure_without_waiting_for_timeout() {
    let fixture = Fixture::new(256 * 1024, 7, false).await;
    let (_temp, store) = setup();
    let job = Job::new(
        &fixture.url,
        Some("retry-observer.bin"),
        store.paths.downloads.clone(),
        Uuid::new_v4().to_string(),
    )
    .unwrap();
    store.enqueue(&job).unwrap();
    let scheduler = Scheduler::start(store.clone()).unwrap();
    let error = completed(&store, &job.id, &fixture, &scheduler)
        .await
        .unwrap_err();
    assert!(error.starts_with("Completion failed;"), "{error}");
    assert!(error.contains("503"), "{error}");
    assert!(error.contains("retry"), "{error}");
    assert_eq!(fixture.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn completion_wait_reports_scheduler_store_error_without_waiting_for_timeout() {
    let fixture = Fixture::new(256 * 1024, 0, false).await;
    let (_temp, store) = setup();
    let job = Job::new(
        &fixture.url,
        Some("write-error.bin"),
        store.paths.downloads.clone(),
        Uuid::new_v4().to_string(),
    )
    .unwrap();
    store.enqueue(&job).unwrap();
    crate::fixture_database::open(store.paths.data.join("queue.sqlite3")).unwrap()
        .execute_batch("CREATE TRIGGER reject_update BEFORE UPDATE ON jobs BEGIN SELECT RAISE(FAIL, 'injected scheduler write failure'); END;").unwrap();
    let scheduler = Scheduler::start(store.clone()).unwrap();
    let error = completed(&store, &job.id, &fixture, &scheduler)
        .await
        .unwrap_err();
    assert!(error.starts_with("Scheduler failed while"), "{error}");
    assert!(
        error.contains("injected scheduler write failure"),
        "{error}"
    );
    assert!(fixture.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn verified_link_queues_normally_and_other_downloads_continue_while_awaiting_confirmation() {
    let fixture = Fixture::new(256 * 1024, 0, false).await;
    let (_temp, store) = setup();
    let original = paused(&store, &fixture.data);
    let scheduler = Scheduler::start(store.clone()).unwrap();
    let (request, _) = checked(&scheduler, &original, &fixture.url).await;
    let other = Job::new(
        &fixture.url,
        Some("other.bin"),
        store.paths.downloads.clone(),
        Uuid::new_v4().to_string(),
    )
    .unwrap();
    store.enqueue(&other).unwrap();
    let other = completed(&store, &other.id, &fixture, &scheduler)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(other.saved_path.unwrap()).unwrap(),
        *fixture.data
    );
    assert_eq!(store.get(&original.id).unwrap().unwrap().url, original.url);
    assert!(matches!(
        apply(&scheduler, request, false).await,
        Event::Applied { .. }
    ));
    scheduler
        .send(Command::StartQueue(
            store.job_queue(&original.id).unwrap().unwrap(),
        ))
        .unwrap();
    let recovered = completed(&store, &original.id, &fixture, &scheduler)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(recovered.saved_path.unwrap()).unwrap(),
        *fixture.data
    );
}

#[tokio::test]
async fn server_changes_after_confirmation_fail_without_appending_unverified_bytes() {
    let fixture = Fixture::new(128 * 1024, 0, false).await;
    let (_temp, store) = setup();
    let original = paused(&store, &fixture.data);
    let before = snapshot(&store, &original).1;
    let scheduler = Scheduler::start(store.clone()).unwrap();
    let (request, _) = checked(&scheduler, &original, &fixture.url).await;
    assert!(matches!(
        apply(&scheduler, request, false).await,
        Event::Applied { .. }
    ));
    fixture.mode.store(2, Ordering::SeqCst);
    drop(scheduler);
    let failed = run(&store, store.get(&original.id).unwrap().unwrap()).await;
    assert_eq!(failed.status, Status::Failed);
    assert!(
        failed
            .error
            .as_ref()
            .unwrap()
            .contains("remote file changed")
    );
    assert_eq!(snapshot(&store, &original).1, before);
    assert!(failed.saved_path.is_none());
}

#[tokio::test]
async fn connection_errors_hide_signed_urls_and_invalid_ids_never_access_cache_paths() {
    let (_temp, store) = setup();
    let mut original = paused(&store, &vec![7; 4096]);
    let scheduler = Scheduler::start(store.clone()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/file?token=hidden-secret",
        listener.local_addr().unwrap()
    );
    drop(listener);
    let before = snapshot(&store, &original);
    let request = begin(&scheduler, &original, &url);
    let Event::Failed { message, .. } = event(&scheduler, request).await else {
        panic!("Unavailable server was accepted");
    };
    assert!(!message.contains("hidden-secret"));
    assert!(!message.contains(&url));
    assert_eq!(snapshot(&store, &original), before);
    original.id = "../outside-cache".into();
    original.request_id = Uuid::new_v4().to_string();
    let error = store.enqueue(&original).unwrap_err();
    assert!(error.to_string().contains("identifier"));
    assert!(!store.paths.data.join("outside-cache").exists());
}

#[tokio::test]
async fn premature_confirmation_releases_the_failed_request_for_a_later_check() {
    let body_gate = Arc::new(tokio::sync::Semaphore::new(0));
    let fixture = Fixture::with_body_gate(1024 * 1024, 0, false, Some(body_gate.clone())).await;
    let (_temp, store) = setup();
    let original = paused(&store, &fixture.data);
    let before = snapshot(&store, &original);
    let scheduler = Scheduler::start(store.clone()).unwrap();
    let request = begin(&scheduler, &original, &fixture.url);
    tokio::time::timeout(PROGRESS_LIMIT, async {
        while fixture.requests.lock().unwrap().len() < 2 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let result = apply(&scheduler, request, false).await;
    assert!(
        matches!(&result, Event::Failed { message, .. } if message.contains("finish")),
        "{result:?}"
    );
    body_gate.add_permits(1);
    // The cancelled read may still be releasing its handle; a new request can
    // report stopping, but must eventually be accepted without app restart.
    tokio::time::timeout(PROGRESS_LIMIT, async {
        loop {
            let next = begin(&scheduler, &original, &fixture.url);
            match event(&scheduler, next).await {
                Event::Checked {
                    outcome: Outcome::Resume { .. },
                    ..
                } => {
                    scheduler
                        .send(Command::Link(LinkCommand::Cancel(next)))
                        .unwrap();
                    break;
                }
                Event::Failed { message, .. } if message.contains("stopping") => {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                other => panic!("Unexpected later check result: {other:?}"),
            }
        }
    })
    .await
    .unwrap();
    drop(scheduler);
    assert_eq!(snapshot(&store, &original), before);
}

#[tokio::test]
async fn link_check_obeys_saved_limits_and_live_removal_wakes_verification() {
    let fixture = Fixture::new(1024 * 1024, 2, false).await;
    let (_temp, store) = setup();
    let original = paused(&store, &fixture.data);
    let before = snapshot(&store, &original);
    store
        .set_bandwidth_limits(&download_engine::bandwidth::Update {
            request: Uuid::new_v4(),
            global: Some(1024),
            download: Some((original.id.clone(), Some(2048))),
        })
        .unwrap();
    let scheduler = Scheduler::start(store.clone()).unwrap();
    let started = std::time::Instant::now();
    let request = begin(&scheduler, &original, &fixture.url);
    tokio::time::sleep(Duration::from_millis(700)).await;
    let events: Vec<_> = scheduler.link_events.try_iter().collect();
    for event in events {
        match event {
            Event::Progress { checked, .. } => {
                assert!(checked as f64 <= started.elapsed().as_secs_f64() * 1024.0 + 64.0)
            }
            other => panic!("Limited check finished too soon: {other:?}"),
        }
    }
    scheduler
        .send(Command::Bandwidth(download_engine::bandwidth::Update {
            request: Uuid::new_v4(),
            global: None,
            download: Some((original.id.clone(), None)),
        }))
        .unwrap();
    assert!(matches!(
        event(&scheduler, request).await,
        Event::Checked {
            outcome: Outcome::Resume { .. },
            ..
        }
    ));
    assert_eq!(snapshot(&store, &original), before);
    scheduler
        .send(Command::Link(LinkCommand::Cancel(request)))
        .unwrap();
}

#[tokio::test]
async fn adaptive_layout_checks_every_saved_part_and_assembles_in_byte_order() {
    use sha2::{Digest, Sha256};
    for corrupt_tail in [false, true] {
        let fixture = Fixture::new(1024 * 1024, 2, false).await;
        let (_temp, store) = setup();
        let mut original = paused(&store, &fixture.data);
        let plan = original.plan.as_mut().unwrap();
        let old_end = plan.segments[0].end;
        let start = old_end.div_ceil(2);
        plan.segments[0].end = start - 1;
        plan.segments.push(Segment {
            start,
            end: old_end,
        });
        let tail_path = store
            .paths
            .parts(&original.id)
            .unwrap()
            .join("segment-4.part");
        let mut tail = fixture.data[start as usize..start as usize + 65536].to_vec();
        if corrupt_tail {
            tail[0] ^= 1;
        }
        std::fs::write(&tail_path, &tail).unwrap();
        original.downloaded += tail.len() as u64;
        original.integrity.expected = Some(format!("{:x}", Sha256::digest(&*fixture.data)));
        store.save(&original).unwrap();
        let before = snapshot(&store, &original);
        let scheduler = Scheduler::start(store.clone()).unwrap();
        let (request, outcome) = checked(&scheduler, &original, &fixture.url).await;
        assert_eq!(snapshot(&store, &original), before);
        assert_eq!(std::fs::read(&tail_path).unwrap(), tail);
        if corrupt_tail {
            assert!(matches!(outcome, Outcome::RestartRequired(_)));
        } else {
            assert_eq!(
                outcome,
                Outcome::Resume {
                    saved: original.downloaded,
                    total: fixture.data.len() as u64
                }
            );
            assert!(matches!(
                apply(&scheduler, request, false).await,
                Event::Applied { .. }
            ));
            let saved = store.get(&original.id).unwrap().unwrap();
            drop(scheduler);
            let finished = run(&store, saved).await;
            assert_eq!(finished.status, Status::Complete, "{:?}", finished.error);
            assert_eq!(finished.integrity.actual, finished.integrity.expected);
            assert_eq!(
                std::fs::read(finished.saved_path.unwrap()).unwrap(),
                *fixture.data
            );
        }
        assert!(
            fixture
                .requests
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.contains(&format!(
                    "range: bytes={start}-{}",
                    start + tail.len() as u64 - 1
                )))
        );
    }
}
