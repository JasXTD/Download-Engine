use super::*;
fn fixture() -> (tempfile::TempDir, Store) {
    let temp = tempfile::tempdir().unwrap();
    let s = Store::open(crate::paths::AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    s.set_queue_running("main", false).unwrap();
    (temp, s)
}
fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[test]
fn pruned_receipt_remains_deduplicated_across_reopen_and_conflicting_retries() {
    let (_temp, store) = fixture();
    let original = id();
    store.submit_control(&original, &Action::PauseAll).unwrap();
    store.cancel_control(&original).unwrap();
    // Exercise the real pruning path instead of deleting the target directly.
    for _ in 0..1005 {
        let key = id();
        store.submit_control(&key, &Action::PauseAll).unwrap();
        store.cancel_control(&key).unwrap();
    }
    let reopened = Store::open(store.paths.clone()).unwrap();
    assert_eq!(
        reopened.control_receipt(&original).unwrap().state,
        "retired"
    );
    assert_eq!(
        reopened
            .submit_control(&original, &Action::PauseAll)
            .unwrap()
            .state,
        "retired"
    );
    assert!(
        reopened
            .submit_control(
                &original,
                &Action::CreateQueue {
                    name: "Duplicate".into()
                }
            )
            .is_err()
    );
    assert!(reopened.claim_control().unwrap().is_none());
    let conn = reopened.connect().unwrap();
    assert!(
        conn.query_row("SELECT count(*) FROM control_requests", [], |r| r
            .get::<_, i64>(0))
            .unwrap()
            <= 1000
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM control_request_ids", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1006
    );
}

#[test]
fn request_identity_cancellation_expiry_and_interrupted_execution_are_explicit() {
    let (_temp, s) = fixture();
    let key = id();
    s.submit_control(&key, &Action::PauseAll).unwrap();
    s.submit_control(&key, &Action::PauseAll).unwrap();
    assert!(
        s.submit_control(
            &key,
            &Action::CreateQueue {
                name: "Other".into()
            }
        )
        .is_err()
    );
    assert_eq!(s.cancel_control(&key).unwrap().state, "cancelled");
    assert!(s.claim_control().unwrap().is_none());
    let expired = id();
    s.submit_control(&expired, &Action::PauseAll).unwrap();
    s.connect()
        .unwrap()
        .execute(
            "UPDATE control_requests SET expires=0 WHERE id=?1",
            [&expired],
        )
        .unwrap();
    assert!(s.claim_control().unwrap().is_none());
    assert_eq!(s.control_receipt(&expired).unwrap().state, "expired");
    let running = id();
    s.submit_control(&running, &Action::PauseAll).unwrap();
    assert_eq!(s.claim_control().unwrap().unwrap().0, running);
    assert_eq!(s.cancel_control(&running).unwrap().state, "running");
    let reopened = Store::open(s.paths.clone()).unwrap();
    reopened.recover_controls().unwrap();
    assert_eq!(
        reopened.control_receipt(&running).unwrap().state,
        "interrupted"
    );
    assert!(reopened.claim_control().unwrap().is_none());
}

#[test]
fn repeated_add_has_stable_job_identity_and_batch_is_all_or_nothing() {
    let (_temp, s) = fixture();
    let key = id();
    let action = Action::Add {
        urls: vec![
            "https://example.com/a.zip".into(),
            "https://example.com/b.zip".into(),
        ],
        queue: "Main queue".into(),
        folder: None,
        name: None,
        script_run: None,
    };
    let a = execute_stored(&s, &key, action.clone()).unwrap();
    let b = execute_stored(&s, &key, action).unwrap();
    assert_eq!(a, b);
    assert_eq!(s.list().unwrap().len(), 2);
    let invalid = Action::Add {
        urls: vec!["https://example.com/c.zip".into(), "file:///local".into()],
        queue: "main".into(),
        folder: None,
        name: None,
        script_run: None,
    };
    assert!(execute_stored(&s, &id(), invalid).is_err());
    assert_eq!(s.list().unwrap().len(), 2);
}

#[test]
fn add_checks_script_identity_at_execution_and_keeps_human_destinations() {
    use crate::script_automation::{Language, Script, Trigger};
    let (temp, s) = fixture();
    let outside = temp.path().join("human-selected-folder");
    let action = |script_run, folder| Action::Add {
        urls: vec!["https://example.invalid/fixture.bin".into()],
        queue: "main".into(),
        folder: Some(folder),
        name: None,
        script_run,
    };
    let script = s
        .save_script(&Script {
            id: id(),
            revision: 0,
            name: "Sample fixture".into(),
            language: Language::Lua,
            body: "-- no execution needed".into(),
            enabled: false,
            trigger: Trigger::QueueCompleted {
                queue: "main".into(),
            },
            timeout_seconds: 30,
            change_files: true,
        })
        .unwrap();
    let sample = s.run_script_sample(&script.id, &id(), &[]).unwrap();
    let error = execute_stored(
        &s,
        &id(),
        action(Some(sample.id), s.paths.downloads.clone()),
    )
    .unwrap_err();
    assert!(error.to_string().contains("Sample"));
    assert!(execute_stored(&s, &id(), action(Some(id()), s.paths.downloads.clone())).is_err());
    assert!(s.list().unwrap().is_empty());
    execute_stored(&s, &id(), action(None, outside.clone())).unwrap();
    assert_eq!(s.list().unwrap()[0].directory, outside);
    assert!(!outside.exists());
    // Saved commands from before this release remain compatible.
    let old: Action = serde_json::from_str(r#"{"action":"add","urls":["https://example.invalid/old.bin"],"queue":"main","folder":null,"name":null}"#).unwrap();
    assert!(old.script_run().is_none());
}

#[test]
fn scheduler_executes_commands_and_retries_return_same_receipt() {
    let (_temp, s) = fixture();
    let scheduler = crate::scheduler::Scheduler::start(s.clone()).unwrap();
    let key = id();
    let action = Action::CreateQueue {
        name: "CLI queue".into(),
    };
    s.submit_control(&key, &action).unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let r = s.control_receipt(&key).unwrap();
        if r.state == "done" {
            break;
        }
        assert!(std::time::Instant::now() < until, "{r:?}");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(s.submit_control(&key, &action).unwrap().state, "done");
    assert_eq!(s.queues().unwrap().len(), 2);
    let missing = id();
    s.submit_control(&missing, &Action::Pause { id: id() })
        .unwrap();
    loop {
        let r = s.control_receipt(&missing).unwrap();
        if r.state == "failed" {
            assert!(r.error.unwrap().contains("not found"));
            break;
        }
        assert!(std::time::Instant::now() < until);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    drop(scheduler);
}

fn settle(s: &Store, action: &Action) -> Receipt {
    let key = id();
    s.submit_control(&key, action).unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let receipt = s.control_receipt(&key).unwrap();
        if !matches!(receipt.state.as_str(), "pending" | "running") {
            return receipt;
        }
        assert!(std::time::Instant::now() < until, "{receipt:?}");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
fn done(receipt: Receipt) {
    assert_eq!(receipt.state, "done", "{receipt:?}");
}
fn refused(receipt: Receipt, needle: &str) {
    assert_eq!(receipt.state, "failed", "{receipt:?}");
    let error = receipt.error.unwrap();
    assert!(error.contains(needle), "{error}");
}
fn finished_file(s: &Store, name: &str) -> Job {
    let job = s
        .new_download(&format!("https://example.com/{name}"), None, None, id())
        .unwrap();
    let mut job = s.enqueue(&job).unwrap();
    std::fs::create_dir_all(&job.directory).unwrap();
    let path = job.directory.join(&job.name);
    std::fs::write(&path, name).unwrap();
    job.status = crate::model::Status::Complete;
    job.saved_path = Some(path);
    s.save(&job).unwrap();
    job
}

#[cfg(windows)]
#[test]
fn script_file_actions_need_permission_and_never_overwrite_or_leave_the_download_folders() {
    use crate::script_automation::{Language, Script, Trigger};
    let (temp, s) = fixture();
    let mut script = s
        .save_script(&Script {
            id: id(),
            revision: 0,
            name: "Sort files".into(),
            language: Language::Lua,
            body: "print('sorting')".into(),
            enabled: false,
            trigger: Trigger::Manual,
            timeout_seconds: 30,
            change_files: false,
        })
        .unwrap();
    let run = s.run_script(&script.id, &id()).unwrap().id;
    let report = finished_file(&s, "report.pdf");
    let other = finished_file(&s, "other.pdf");
    let scheduler = crate::scheduler::Scheduler::start(s.clone()).unwrap();
    let rename = |name: &str| Action::Rename {
        id: report.id.clone(),
        name: name.into(),
        script_run: Some(run.clone()),
    };
    refused(
        settle(&s, &rename("renamed.pdf")),
        "Can change downloaded files",
    );
    assert!(report.saved_path.as_ref().unwrap().is_file());
    script.change_files = true;
    s.save_script(&script).unwrap();
    done(settle(&s, &rename("Renamed report.pdf")));
    let renamed = s.get(&report.id).unwrap().unwrap();
    assert_eq!(renamed.name, "Renamed report.pdf");
    let path = renamed.saved_path.clone().unwrap();
    assert_eq!(path, report.directory.join("Renamed report.pdf"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "report.pdf");
    assert!(!report.saved_path.as_ref().unwrap().exists());
    refused(settle(&s, &rename("other.pdf")), "already exists");
    assert_eq!(
        std::fs::read_to_string(other.saved_path.as_ref().unwrap()).unwrap(),
        "other.pdf"
    );
    assert!(path.is_file());
    refused(settle(&s, &rename("bad:name.pdf")), "Choose a file name");
    refused(settle(&s, &rename("CON")), "Choose a file name");
    let to = |folder: std::path::PathBuf| Action::MoveFile {
        id: report.id.clone(),
        folder,
        script_run: Some(run.clone()),
    };
    let sorted = s.paths.downloads.join("Documents").join("2026");
    done(settle(&s, &to(sorted.clone())));
    let moved = s.get(&report.id).unwrap().unwrap();
    assert_eq!(moved.directory, sorted);
    assert_eq!(
        moved.saved_path.as_deref(),
        Some(sorted.join("Renamed report.pdf").as_path())
    );
    assert!(!path.exists());
    let outside = temp.path().join("elsewhere");
    refused(
        settle(&s, &to(outside.clone())),
        "only into your download folder",
    );
    assert!(!outside.exists());
    refused(
        settle(&s, &to(s.paths.downloads.join("..").join("x"))),
        "normalized",
    );
    let remove = Action::Remove {
        id: report.id.clone(),
        delete_file: true,
        script_run: Some(run.clone()),
    };
    done(settle(&s, &remove));
    assert!(s.get(&report.id).unwrap().is_none());
    assert!(!sorted.join("Renamed report.pdf").exists());
    // People using the CLI need no switch.
    let mut off = s.script(&script.id).unwrap();
    off.change_files = false;
    s.save_script(&off).unwrap();
    let human = Action::Remove {
        id: other.id.clone(),
        delete_file: false,
        script_run: None,
    };
    done(settle(&s, &human));
    assert!(other.saved_path.as_ref().unwrap().is_file());
    drop(scheduler);
}

#[test]
fn script_retries_are_limited_and_notifications_reach_the_desktop() {
    use crate::script_automation::{Language, MAX_NOTICES, MAX_SCRIPT_RETRIES, Script, Trigger};
    let (_temp, s) = fixture();
    let script = s
        .save_script(&Script {
            id: id(),
            revision: 0,
            name: "Retry failed downloads".into(),
            language: Language::Lua,
            body: "print('retrying')".into(),
            enabled: false,
            trigger: Trigger::Manual,
            timeout_seconds: 30,
            change_files: false,
        })
        .unwrap();
    let run = s.run_script(&script.id, &id()).unwrap().id;
    // A stopped queue of its own: resuming in Main would start the download.
    let held = s.create_queue("Held").unwrap();
    let job = s
        .new_download("https://example.com/archive.zip", None, None, id())
        .unwrap();
    let mut job = s.enqueue_batch(&[job], &held.id).unwrap().remove(0);
    let fail = |job: &mut Job| {
        job.status = crate::model::Status::Failed;
        job.error = Some("Server returned 404".into());
        s.save(job).unwrap();
    };
    fail(&mut job);
    let scheduler = crate::scheduler::Scheduler::start(s.clone()).unwrap();
    // Starting the scheduler interrupts old runs; this one keeps running here.
    s.connect()
        .unwrap()
        .execute("UPDATE script_runs SET state='running' WHERE id=?1", [&run])
        .unwrap();
    let retry = Action::Retry {
        id: job.id.clone(),
        script_run: Some(run.clone()),
    };
    for _ in 0..MAX_SCRIPT_RETRIES {
        done(settle(&s, &retry));
        let mut current = s.get(&job.id).unwrap().unwrap();
        assert_eq!(current.status, crate::model::Status::Queued);
        fail(&mut current);
    }
    refused(settle(&s, &retry), "already retried");
    // A refused retry is not counted, and people can always retry.
    let person = Action::Retry {
        id: job.id.clone(),
        script_run: None,
    };
    done(settle(&s, &person));
    // Now queued again: retry is only for downloads that need attention.
    refused(settle(&s, &person), "need attention");
    let notify = |title: &str| Action::Notify {
        title: title.into(),
        text: "Holiday photos.zip still needs attention.".into(),
        script_run: run.clone(),
    };
    for i in 0..MAX_NOTICES {
        done(settle(&s, &notify(&format!("Notice {i}"))));
    }
    refused(settle(&s, &notify("One too many")), "at most 5");
    let shown: Vec<_> = scheduler.script_notices.try_iter().collect();
    assert_eq!(shown.len(), MAX_NOTICES as usize);
    assert_eq!(shown[0].title, "Notice 0");
    assert_eq!(shown[0].script, "Retry failed downloads");
    assert_eq!(shown[0].run, run);
    assert!(
        shown[0]
            .text()
            .starts_with("Holiday photos.zip still needs attention.\nRetry failed downloads · ")
    );
    drop(scheduler);
}

#[test]
fn sample_runs_describe_what_they_would_do() {
    let (_temp, s) = fixture();
    let job = s
        .new_download("https://example.com/archive.zip", None, None, id())
        .unwrap();
    let job = s.enqueue(&job).unwrap();
    for (action, expected) in [
        (
            Action::Retry {
                id: job.id.clone(),
                script_run: None,
            },
            "Would retry archive.zip (sample run, nothing changed)",
        ),
        (
            Action::Remove {
                id: job.id.clone(),
                delete_file: true,
                script_run: None,
            },
            "Would remove archive.zip and delete its file (sample run, nothing changed)",
        ),
        (
            Action::Notify {
                title: "Done".into(),
                text: "All files sorted".into(),
                script_run: id(),
            },
            "Would show the notification Done · All files sorted (sample run, nothing changed)",
        ),
    ] {
        assert_eq!(simulated(&s, &action), expected);
    }
    // Commands saved before 1.22 still read, without a script run.
    let legacy: Action = serde_json::from_str(r#"{"action":"retry","id":"x"}"#).unwrap();
    assert!(legacy.script_run().is_none());
    assert_eq!(clip("  long title  ", 4), "long");
    assert_eq!(clip("ab ", 64), "ab");
}
