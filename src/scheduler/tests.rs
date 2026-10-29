use super::*;
use crate::{paths::AppPaths, queue::MAIN_QUEUE};

#[test]
fn clear_history_rechecks_status_and_queue_when_the_command_executes() {
    let (_root, store) = setup();
    let mut job = add(&store, MAIN_QUEUE, 1).remove(0);
    let mut active = HashMap::new();
    let command = || Command::ClearHistory {
        id: job.id.clone(),
        queue: MAIN_QUEUE.into(),
    };
    let parts = store.paths.parts(&job.id).unwrap();
    std::fs::create_dir_all(&parts).unwrap();
    let partial = parts.join("0.part");
    std::fs::write(&partial, b"unfinished progress").unwrap();
    for status in [
        Status::Paused,
        Status::Queued,
        Status::Downloading,
        Status::Failed,
    ] {
        job.status = status;
        store.save(&job).unwrap();
        store.mark_queue_pause(&job.id, true).unwrap();
        apply(&store, &mut active, command()).unwrap();
        assert_eq!(store.get(&job.id).unwrap().unwrap().status, status);
        assert!(store.queue_paused_jobs().unwrap().contains(&job.id));
        assert_eq!(std::fs::read(&partial).unwrap(), b"unfinished progress");
    }
    job.status = Status::Complete;
    // Even a saved file named like an old staging file is user data here.
    let file = job.directory.join(format!(".idm-ex-{}.assembling", job.id));
    std::fs::write(&file, b"completed bytes").unwrap();
    job.saved_path = Some(file.clone());
    store.save(&job).unwrap();
    let other = store.create_queue("Moved").unwrap();
    store.move_job(&job.id, &other.id).unwrap();
    apply(&store, &mut active, command()).unwrap();
    assert!(store.get(&job.id).unwrap().is_some());
    store.move_job(&job.id, MAIN_QUEUE).unwrap();
    apply(&store, &mut active, command()).unwrap();
    assert!(store.get(&job.id).unwrap().is_none());
    assert_eq!(std::fs::read(file).unwrap(), b"completed bytes");
    apply(&store, &mut active, command()).unwrap();
}

#[test]
fn restart_clears_both_contiguous_and_out_of_order_video_receipts() {
    let (_temp, store) = setup();
    let mut job = add(&store, MAIN_QUEUE, 1).remove(0);
    job.hls = Some(crate::hls::Download::new(crate::hls::Playlist {
        public_only: false,
        source: job.url.clone(),
        url: job.url.clone(),
        quality: "Test".into(),
        bandwidth: None,
        duration_ms: 3000,
        sequence: 0,
        format: crate::hls::Format::TransportStream,
        parts: (0..3)
            .map(|index| crate::hls::Part {
                url: format!("https://example.com/{index}.ts"),
                range: None,
                initialization: false,
            })
            .collect(),
        audio: None,
        dash: None,
    }));
    let receipt = crate::hls::CompletedPart {
        size: 188,
        sha256: "ab".repeat(32),
    };
    let download = job.hls.as_mut().unwrap();
    download.remember_part(0, receipt.clone()).unwrap();
    download.remember_part(2, receipt).unwrap();
    job.downloaded = 376;
    store.save(&job).unwrap();
    apply(
        &store,
        &mut HashMap::new(),
        Command::Restart(job.id.clone()),
    )
    .unwrap();
    let restarted = store.get(&job.id).unwrap().unwrap();
    assert_eq!(restarted.downloaded, 0);
    let download = restarted.hls.unwrap();
    assert!(download.completed.is_empty());
    assert!(download.completed_ahead.is_empty());
}

#[tokio::test]
async fn worker_error_never_leaves_an_ownerless_downloading_job() {
    let (_temp, store) = setup();
    let mut job = add(&store, MAIN_QUEUE, 1).remove(0);
    job.status = Status::Downloading;
    store.save(&job).unwrap();
    let mut active = HashMap::from([(
        job.id.clone(),
        Active {
            token: CancellationToken::new(),
            queue: MAIN_QUEUE.into(),
            user_paused: false,
        },
    )]);
    let mut tasks = JoinSet::new();
    let id = job.id.clone();
    // An engine initialization/final-save error can escape before it saves
    // its terminal state. This is the result the scheduler actually receives.
    let worker = tasks.spawn(async move { (id, Err(anyhow::anyhow!("Fixture save failed"))) });
    let mut worker_ids = HashMap::from([(worker.id(), job.id.clone())]);
    let (errors_tx, errors) = mpsc::channel();
    let (finished_tx, finished) = mpsc::channel();
    finish_worker(
        &store,
        &mut active,
        &mut worker_ids,
        tasks.join_next_with_id().await.unwrap(),
        &errors_tx,
        &finished_tx,
    );
    let saved = store.get(&job.id).unwrap().unwrap();
    assert_eq!(saved.status, Status::Failed);
    assert!(saved.error.unwrap().contains("Fixture save failed"));
    assert!(saved.retry.next_at.is_none());
    assert!(active.is_empty() && worker_ids.is_empty());
    assert_eq!(finished.try_iter().count(), 1);
    assert!(errors.try_iter().any(|e| e.contains("Fixture save failed")));
    apply(&store, &mut active, Command::Resume(job.id.clone())).unwrap();
    assert_eq!(store.get(&job.id).unwrap().unwrap().status, Status::Queued);
}

#[tokio::test]
async fn panic_after_durable_completion_does_not_erase_success() {
    let (_temp, store) = setup();
    let mut job = add(&store, MAIN_QUEUE, 1).remove(0);
    job.status = Status::Complete;
    store.save(&job).unwrap();
    let mut active = HashMap::from([(
        job.id.clone(),
        Active {
            token: CancellationToken::new(),
            queue: MAIN_QUEUE.into(),
            user_paused: false,
        },
    )]);
    let mut tasks = JoinSet::<(String, Result<()>)>::new();
    let worker = tasks.spawn(async { panic!("Fixture cleanup panic") });
    let mut worker_ids = HashMap::from([(worker.id(), job.id.clone())]);
    let (errors, _) = mpsc::channel();
    let (finished_tx, finished) = mpsc::channel();
    finish_worker(
        &store,
        &mut active,
        &mut worker_ids,
        tasks.join_next_with_id().await.unwrap(),
        &errors,
        &finished_tx,
    );
    assert_eq!(
        store.get(&job.id).unwrap().unwrap().status,
        Status::Complete
    );
    assert!(active.is_empty() && worker_ids.is_empty());
    assert_eq!(finished.try_iter().next().unwrap().status, Status::Complete);
}

#[test]
fn restart_and_remove_refuse_unexpected_cache_directories() {
    for remove in [false, true] {
        let (_temp, store) = setup();
        let mut job = add(&store, MAIN_QUEUE, 1).remove(0);
        job.status = Status::Paused;
        store.save(&job).unwrap();
        let unexpected = store.paths.parts(&job.id).unwrap().join("unexpected");
        std::fs::create_dir_all(&unexpected).unwrap();
        let marker = unexpected.join("keep.txt");
        std::fs::write(&marker, b"not a partial file").unwrap();
        let command = if remove {
            Command::Remove(job.id.clone())
        } else {
            Command::Restart(job.id.clone())
        };
        assert!(apply(&store, &mut HashMap::new(), command).is_err());
        assert_eq!(std::fs::read(marker).unwrap(), b"not a partial file");
        assert_eq!(store.get(&job.id).unwrap().unwrap().status, Status::Paused);
    }
}

fn setup() -> (tempfile::TempDir, Store) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("files"),
    })
    .unwrap();
    (temp, store)
}

#[cfg(windows)]
#[test]
fn slow_file_copy_keeps_commands_receipts_and_shutdown_responsive() {
    use crate::{control::Action, file_relocation::tests::Gate};
    let (_temp, store) = setup();
    store.set_queue_running(MAIN_QUEUE, false).unwrap();
    let mut jobs = add(&store, MAIN_QUEUE, 2);
    store.set_queue_running(MAIN_QUEUE, false).unwrap();
    let mut complete = jobs.remove(0);
    let waiting = jobs.remove(0);
    let original = store.paths.downloads.join("completed.bin");
    std::fs::write(&original, vec![0x67; 700_000]).unwrap();
    complete.status = Status::Complete;
    complete.saved_path = Some(original.clone());
    store.save(&complete).unwrap();
    let gate = Gate::copying(&complete.id);
    let scheduler = Scheduler::start(store.clone()).unwrap();
    let moving = uuid::Uuid::new_v4().to_string();
    let script = store
        .save_script(&crate::script_automation::Script {
            id: uuid::Uuid::new_v4().to_string(),
            revision: 0,
            name: "Controlled timeout".into(),
            language: crate::script_automation::Language::PowerShell,
            body: "Start-Sleep 60".into(),
            enabled: false,
            trigger: crate::script_automation::Trigger::Manual,
            timeout_seconds: 1,
            change_files: false,
        })
        .unwrap();
    let script_run = store
        .run_script(&script.id, &uuid::Uuid::new_v4().to_string())
        .unwrap();
    store
        .submit_control(
            &moving,
            &Action::Rename {
                id: complete.id.clone(),
                name: "moved.bin".into(),
                script_run: None,
            },
        )
        .unwrap();
    gate.wait();
    assert_eq!(store.control_receipt(&moving).unwrap().state, "running");
    let until = std::time::Instant::now() + Duration::from_secs(5);
    while store.script_run(&script_run.id).unwrap().active() {
        assert!(
            std::time::Instant::now() < until,
            "Script deadline was blocked by the copy"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(store.script_run(&script_run.id).unwrap().state, "timed_out");

    let rejected = uuid::Uuid::new_v4().to_string();
    let pause = uuid::Uuid::new_v4().to_string();
    store
        .submit_control(
            &rejected,
            &Action::Remove {
                id: complete.id.clone(),
                delete_file: true,
                script_run: None,
            },
        )
        .unwrap();
    store
        .submit_control(
            &pause,
            &Action::Pause {
                id: waiting.id.clone(),
            },
        )
        .unwrap();
    let until = std::time::Instant::now() + Duration::from_secs(3);
    while store.control_receipt(&pause).unwrap().state != "done" {
        assert!(
            std::time::Instant::now() < until,
            "Unrelated pause blocked by the copy"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        store.get(&waiting.id).unwrap().unwrap().status,
        Status::Paused
    );
    assert_eq!(store.control_receipt(&rejected).unwrap().state, "failed");
    assert_eq!(store.control_receipt(&moving).unwrap().state, "running");
    // No gate release: shutdown must reach and cancel the copy itself.
    let (finished, received) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        drop(scheduler);
        let _ = finished.send(());
    });
    let outcome = received.recv_timeout(Duration::from_secs(3));
    gate.release();
    thread.join().unwrap();
    assert!(outcome.is_ok(), "Shutdown waited for the full copy");
    assert_eq!(store.control_receipt(&moving).unwrap().state, "failed");
    assert_eq!(std::fs::read(&original).unwrap(), vec![0x67; 700_000]);
    assert!(!store.paths.downloads.join("moved.bin").exists());
    assert!(crate::file_relocation::pending(&store).unwrap().is_empty());
    assert_eq!(
        store.get(&complete.id).unwrap().unwrap().saved_path,
        Some(original)
    );
}

#[cfg(windows)]
#[test]
fn delete_file_removes_only_the_confirmed_file_and_its_history() {
    let (_temp, store) = setup();
    let mut job = add(&store, MAIN_QUEUE, 1).remove(0);
    let saved = store.paths.downloads.join("completed.bin");
    let other = store.paths.downloads.join("keep.bin");
    std::fs::write(&saved, b"completed").unwrap();
    std::fs::hard_link(&saved, &other).unwrap();
    job.status = Status::Complete;
    job.saved_path = Some(saved.clone());
    store.save(&job).unwrap();
    apply(
        &store,
        &mut HashMap::new(),
        Command::DeleteFile {
            id: job.id.clone(),
            path: saved.clone(),
        },
    )
    .unwrap();
    assert!(!saved.exists());
    assert!(store.get(&job.id).unwrap().is_none());
    assert_eq!(std::fs::read(other).unwrap(), b"completed");
}

#[cfg(windows)]
#[test]
fn delete_file_failures_keep_history_and_never_delete_a_folder_or_a_different_path() {
    for case in [
        "missing",
        "locked",
        "folder",
        "changed",
        "unfinished",
        "active",
    ] {
        let (_temp, store) = setup();
        let mut job = add(&store, MAIN_QUEUE, 1).remove(0);
        let saved = store.paths.downloads.join("completed.bin");
        let requested = store.paths.downloads.join("other.bin");
        std::fs::write(&requested, b"other bytes").unwrap();
        if case == "folder" {
            std::fs::create_dir(&saved).unwrap();
            std::fs::write(saved.join("keep.bin"), b"child bytes").unwrap();
        } else if case != "missing" {
            std::fs::write(&saved, b"completed bytes").unwrap();
        }
        job.status = if case == "unfinished" {
            Status::Paused
        } else {
            Status::Complete
        };
        job.saved_path = Some(saved.clone());
        store.save(&job).unwrap();
        let _lock = if case == "locked" {
            use std::os::windows::fs::OpenOptionsExt;
            Some(
                std::fs::OpenOptions::new()
                    .read(true)
                    .share_mode(1)
                    .open(&saved)
                    .unwrap(),
            )
        } else {
            None
        };
        let mut active = HashMap::new();
        if case == "active" {
            active.insert(
                job.id.clone(),
                Active {
                    token: CancellationToken::new(),
                    queue: MAIN_QUEUE.into(),
                    user_paused: false,
                },
            );
        }
        let error = apply(
            &store,
            &mut active,
            Command::DeleteFile {
                id: job.id.clone(),
                path: if case == "changed" {
                    requested.clone()
                } else {
                    saved.clone()
                },
            },
        )
        .unwrap_err();
        assert!(!error.to_string().is_empty(), "{case}");
        assert!(store.get(&job.id).unwrap().is_some(), "{case}");
        assert_eq!(std::fs::read(&requested).unwrap(), b"other bytes", "{case}");
        if case == "folder" {
            assert_eq!(
                std::fs::read(saved.join("keep.bin")).unwrap(),
                b"child bytes"
            );
        } else if case != "missing" {
            assert_eq!(std::fs::read(&saved).unwrap(), b"completed bytes", "{case}");
        }
    }
}

#[test]
fn quota_hold_preserves_manual_pause_even_after_worker_publishes() {
    let (_temp, store) = setup();
    let mut job = Job::new(
        "https://example.com/a.bin",
        None,
        store.paths.downloads.clone(),
        "quota-pause".into(),
    )
    .unwrap();
    job.quota_waiting = true;
    store.enqueue(&job).unwrap();
    let mut active = HashMap::from([(
        job.id.clone(),
        Active {
            token: CancellationToken::new(),
            queue: "main".into(),
            user_paused: false,
        },
    )]);
    apply(&store, &mut active, Command::Pause(job.id.clone())).unwrap();
    settle_worker(&store, &active[&job.id], &job.id).unwrap();
    let saved = store.get(&job.id).unwrap().unwrap();
    assert_eq!(saved.status, Status::Paused);
    assert!(!saved.quota_waiting);
    assert!(saved.error.is_none());
}
#[test]
fn quota_nonresumable_hold_is_not_requeued_by_automatic_queue_ownership() {
    let (_temp, store) = setup();
    let mut job = Job::new(
        "https://example.com/a.bin",
        None,
        store.paths.downloads.clone(),
        "quota-manual".into(),
    )
    .unwrap();
    job.status = Status::Paused;
    job.quota_waiting = true;
    job.downloaded = 100;
    job.error = Some("Data quota reached. Manual restart is required.".into());
    store.enqueue(&job).unwrap();
    store.mark_queue_pause(&job.id, true).unwrap();
    assert!(
        ready_jobs(&store, &HashMap::new(), &mut 0)
            .unwrap()
            .is_empty()
    );
    let saved = store.get(&job.id).unwrap().unwrap();
    assert_eq!(saved.status, Status::Paused);
    assert!(saved.quota_waiting);
    assert_eq!(saved.error, job.error);
    assert!(store.queue_paused_jobs().unwrap().is_empty());
}
#[tokio::test]
async fn terminal_controls_wait_for_worker_release() {
    use std::{future::Future, task::Poll};
    for status in [Status::Paused, Status::Failed, Status::Complete] {
        let (_temp, store) = setup();
        let mut job = Job::new(
            "https://example.com/terminal.bin",
            None,
            store.paths.downloads.clone(),
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
        job.status = status;
        store.enqueue(&job).unwrap();
        let mut active = HashMap::from([(
            job.id.clone(),
            Active {
                token: CancellationToken::new(),
                queue: "main".into(),
                user_paused: false,
            },
        )]);
        // The engine has durably saved its terminal state, but still owns
        // cleanup. Hold that exact boundary without sleeps or wall-clock races.
        let (release, cleanup) = tokio::sync::oneshot::channel::<()>();
        let mut tasks = JoinSet::new();
        let id = job.id.clone();
        let worker = tasks.spawn(async move {
            cleanup.await.unwrap();
            (id, Ok(()))
        });
        let mut worker_ids = HashMap::from([(worker.id(), job.id.clone())]);
        let (errors_tx, errors) = mpsc::channel();
        let (finished_tx, finished) = mpsc::channel();
        let command = if status == Status::Complete {
            Command::Remove(job.id.clone())
        } else {
            Command::Resume(job.id.clone())
        };
        let command = async {
            settle_before_command(
                &store,
                &command,
                &mut active,
                &mut tasks,
                &mut worker_ids,
                &errors_tx,
                &finished_tx,
            )
            .await;
            apply(&store, &mut active, command)
        };
        tokio::pin!(command);
        std::future::poll_fn(|cx| {
            if let Poll::Ready(result) = command.as_mut().poll(cx) {
                panic!("Terminal control ran before worker release: {result:?}");
            }
            Poll::Ready(())
        })
        .await;
        assert_eq!(store.get(&job.id).unwrap().unwrap().status, status);
        release.send(()).unwrap();
        command.await.unwrap();
        assert!(errors.try_iter().collect::<Vec<_>>().is_empty());
        if status == Status::Complete {
            assert!(store.get(&job.id).unwrap().is_none());
        } else {
            assert_eq!(store.get(&job.id).unwrap().unwrap().status, Status::Queued);
        }
        // Notifications must still describe the finished attempt, before
        // Resume/Remove changes or removes the stored row.
        assert_eq!(
            finished.try_iter().count(),
            usize::from(status != Status::Paused)
        );
    }
}

#[tokio::test]
async fn running_download_controls_still_require_pause() {
    let (_temp, store) = setup();
    let mut job = Job::new(
        "https://example.com/active.bin",
        None,
        store.paths.downloads.clone(),
        uuid::Uuid::new_v4().to_string(),
    )
    .unwrap();
    job.status = Status::Downloading;
    store.enqueue(&job).unwrap();
    let mut active = HashMap::from([(
        job.id.clone(),
        Active {
            token: CancellationToken::new(),
            queue: "main".into(),
            user_paused: false,
        },
    )]);
    let mut tasks = JoinSet::new();
    let id = job.id.clone();
    let worker = tasks.spawn(async move {
        std::future::pending::<()>().await;
        (id, Ok(()))
    });
    let mut worker_ids = HashMap::from([(worker.id(), job.id.clone())]);
    let (errors, _) = mpsc::channel();
    let (finished, _) = mpsc::channel();
    let command = Command::Remove(job.id.clone());
    // This must not wait for a still-running transfer before rejecting Remove.
    tokio::time::timeout(
        Duration::from_secs(1),
        settle_before_command(
            &store,
            &command,
            &mut active,
            &mut tasks,
            &mut worker_ids,
            &errors,
            &finished,
        ),
    )
    .await
    .unwrap();
    assert!(
        apply(&store, &mut active, command)
            .unwrap_err()
            .to_string()
            .contains("Pause")
    );
    assert_eq!(
        store.get(&job.id).unwrap().unwrap().status,
        Status::Downloading
    );
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}

fn add(store: &Store, queue: &str, count: usize) -> Vec<Job> {
    let jobs: Vec<_> = (0..count)
        .map(|i| {
            Job::new(
                &format!("https://example.com/{i}.bin"),
                None,
                store.paths.downloads.clone(),
                uuid::Uuid::new_v4().to_string(),
            )
            .unwrap()
        })
        .collect();
    store.enqueue_batch(&jobs, queue).unwrap()
}

fn schedule_time(value: &str) -> i64 {
    use chrono::TimeZone;
    chrono::Local
        .from_local_datetime(
            &chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M").unwrap(),
        )
        .single()
        .unwrap()
        .timestamp_millis()
}

#[test]
fn recurring_stop_waits_for_worker_release_and_preserves_individual_pauses() {
    let (_temp, store) = setup();
    let before = schedule_time("2026-10-12 22:00");
    let start = schedule_time("2026-10-12 23:00");
    let stop = schedule_time("2026-10-13 07:00");
    store.set_queue_limit(MAIN_QUEUE, 2).unwrap();
    store
        .schedule_recurring_queue(
            MAIN_QUEUE,
            &RecurringSchedule::new(1, "23:00", "07:00").unwrap(),
            before / 1000,
        )
        .unwrap();
    let jobs = add(&store, MAIN_QUEUE, 3);
    let mut active = HashMap::new();
    assert!(
        ready_jobs_at(&store, &active, &mut 0, before)
            .unwrap()
            .is_empty()
    );
    let ready = ready_jobs_at(&store, &active, &mut 0, start).unwrap();
    assert_eq!(ready.len(), 2);
    for (queue, mut job) in ready {
        job.status = Status::Downloading;
        job.downloaded = 1234;
        store.save(&job).unwrap();
        active.insert(
            job.id,
            Active {
                token: CancellationToken::new(),
                queue,
                user_paused: false,
            },
        );
    }
    let part = store.paths.parts(&jobs[0].id).unwrap();
    std::fs::create_dir_all(&part).unwrap();
    let bytes = vec![42; 1234];
    std::fs::write(part.join("segment-0.part"), &bytes).unwrap();
    apply(&store, &mut active, Command::Pause(jobs[1].id.clone())).unwrap();
    assert!(
        ready_jobs_at(&store, &active, &mut 0, stop)
            .unwrap()
            .is_empty()
    );
    assert!(active.values().all(|worker| worker.token.is_cancelled()));
    assert_eq!(
        store.get(&jobs[0].id).unwrap().unwrap().status,
        Status::Downloading
    );
    // The next start cannot replace a worker that is still cancelling.
    let next = schedule_time("2026-10-19 23:00");
    assert!(
        ready_jobs_at(&store, &active, &mut 0, next)
            .unwrap()
            .is_empty()
    );
    for job in &jobs[..2] {
        let mut paused = store.get(&job.id).unwrap().unwrap();
        paused.status = Status::Paused;
        store.save(&paused).unwrap();
    }
    active.clear();
    assert!(
        ready_jobs_at(&store, &active, &mut 0, stop)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store.get(&jobs[0].id).unwrap().unwrap().status,
        Status::Queued
    );
    assert_eq!(
        store.get(&jobs[1].id).unwrap().unwrap().status,
        Status::Paused
    );
    let reopened = Store::open(store.paths.clone()).unwrap();
    reopened.recover().unwrap();
    let ready = ready_jobs_at(&reopened, &active, &mut 0, next).unwrap();
    assert_eq!(
        ready
            .iter()
            .map(|(_, job)| job.id.clone())
            .collect::<Vec<_>>(),
        vec![jobs[0].id.clone(), jobs[2].id.clone()]
    );
    assert_eq!(reopened.get(&jobs[0].id).unwrap().unwrap().downloaded, 1234);
    assert_eq!(std::fs::read(part.join("segment-0.part")).unwrap(), bytes);
}

#[test]
fn recurring_recovery_distinguishes_automatic_transfer_from_manual_intent() {
    let (_temp, store) = setup();
    let at = schedule_time("2026-10-12 01:00");
    store
        .schedule_recurring_queue(
            MAIN_QUEUE,
            &RecurringSchedule::new(127, "00:00", "07:00").unwrap(),
            at / 1000,
        )
        .unwrap();
    let jobs = add(&store, MAIN_QUEUE, 2);
    let mut active = HashMap::new();
    for (queue, mut job) in ready_jobs_at(&store, &active, &mut 0, at).unwrap() {
        job.status = Status::Downloading;
        store.save(&job).unwrap();
        active.insert(
            job.id,
            Active {
                token: CancellationToken::new(),
                queue,
                user_paused: false,
            },
        );
    }
    // Crash after the user pause, before the worker can publish Paused.
    apply(&store, &mut active, Command::Pause(jobs[1].id.clone())).unwrap();
    let reopened = Store::open(store.paths.clone()).unwrap();
    reopened.recover().unwrap();
    assert_eq!(
        reopened.get(&jobs[0].id).unwrap().unwrap().status,
        Status::Queued
    );
    assert_eq!(
        reopened.get(&jobs[1].id).unwrap().unwrap().status,
        Status::Paused
    );
    active.clear();
    for (queue, mut job) in ready_jobs_at(&reopened, &active, &mut 0, at).unwrap() {
        job.status = Status::Downloading;
        reopened.save(&job).unwrap();
        active.insert(
            job.id,
            Active {
                token: CancellationToken::new(),
                queue,
                user_paused: false,
            },
        );
    }
    // Switching to manual running also removes automatic recovery ownership.
    apply(
        &reopened,
        &mut active,
        Command::StartQueue(MAIN_QUEUE.into()),
    )
    .unwrap();
    reopened.recover().unwrap();
    assert_eq!(
        reopened.get(&jobs[0].id).unwrap().unwrap().status,
        Status::Paused
    );
    assert!(!reopened.queues().unwrap()[0].automatic());
    reopened.apply_queue_schedules(at / 1000 + 86400).unwrap();
    assert!(reopened.queues().unwrap()[0].running);
}

#[test]
fn pause_all_keeps_a_settling_queue_pause_manual_after_restart() {
    for worker_still_owned in [false, true] {
        let (_temp, store) = setup();
        let mut job = add(&store, MAIN_QUEUE, 1).remove(0);
        job.status = Status::Downloading;
        job.downloaded = 1234;
        store.save(&job).unwrap();
        let mut active = HashMap::from([(
            job.id.clone(),
            Active {
                token: CancellationToken::new(),
                queue: MAIN_QUEUE.into(),
                user_paused: false,
            },
        )]);
        apply(&store, &mut active, Command::PauseQueue(MAIN_QUEUE.into())).unwrap();
        // The engine has saved Paused, but the next scheduler tick has not yet
        // reconciled the automatic pause marker. Pause all must win here too.
        job.status = Status::Paused;
        store.save(&job).unwrap();
        if !worker_still_owned {
            active.clear();
        }
        apply(&store, &mut active, Command::PauseAll).unwrap();
        if let Some(worker) = active.remove(&job.id) {
            assert!(worker.user_paused);
            settle_worker(&store, &worker, &job.id).unwrap();
        }
        let reopened = Store::open(store.paths.clone()).unwrap();
        reopened.recover().unwrap();
        apply(
            &reopened,
            &mut active,
            Command::StartQueue(MAIN_QUEUE.into()),
        )
        .unwrap();
        assert!(ready_jobs(&reopened, &active, &mut 0).unwrap().is_empty());
        let saved = reopened.get(&job.id).unwrap().unwrap();
        assert_eq!(saved.status, Status::Paused);
        assert_eq!(saved.downloaded, 1234);
        assert!(reopened.queue_paused_jobs().unwrap().is_empty());
        apply(&reopened, &mut active, Command::Resume(job.id.clone())).unwrap();
        assert_eq!(
            ready_jobs(&reopened, &active, &mut 0).unwrap()[0].1.id,
            job.id
        );
    }
}

#[test]
fn dispatch_respects_order_queue_limits_and_global_cap() {
    let (_temp, store) = setup();
    let a = add(&store, MAIN_QUEUE, MAX_ACTIVE + 2);
    store.set_queue_limit(MAIN_QUEUE, MAX_ACTIVE).unwrap();
    let second = store.create_queue("Second").unwrap();
    let b = add(&store, &second.id, 4);
    store.set_queue_limit(&second.id, 3).unwrap();
    store.set_queue_running(&second.id, true).unwrap();
    let waiting = store.create_queue("Waiting").unwrap();
    add(&store, &waiting.id, 2);
    store.reorder_job(&a[1].id, true).unwrap();
    let ready = ready_jobs(&store, &HashMap::new(), &mut 0).unwrap();
    assert_eq!(ready.len(), MAX_ACTIVE);
    assert_eq!(ready[0].1.id, a[1].id);
    assert_eq!(ready[1].1.id, b[0].id);
    assert_eq!(ready.iter().filter(|(q, _)| q == &second.id).count(), 3);
    assert!(ready.iter().all(|(q, _)| q != &waiting.id));
    let active: HashMap<_, _> = ready
        .into_iter()
        .map(|(queue, job)| {
            (
                job.id,
                Active {
                    queue,
                    token: CancellationToken::new(),
                    user_paused: false,
                },
            )
        })
        .collect();
    assert!(ready_jobs(&store, &active, &mut 0).unwrap().is_empty());
}

#[test]
fn immediate_start_bypasses_only_the_selected_files_queue_and_preserves_progress() {
    let (_root, store) = setup();
    let queue = store.create_queue("Later").unwrap();
    let now = chrono::Utc::now().timestamp();
    store.schedule_queue(&queue.id, now + 3600, now).unwrap();
    let jobs = add(&store, &queue.id, 3);
    let mut target = jobs[1].clone();
    target.status = Status::Paused;
    target.downloaded = 1234;
    target.total = Some(9876);
    store.save(&target).unwrap();
    let before_queue = store
        .queues()
        .unwrap()
        .into_iter()
        .find(|q| q.id == queue.id)
        .unwrap();
    let before_order = store
        .queue_jobs(&queue.id)
        .unwrap()
        .into_iter()
        .map(|j| j.id)
        .collect::<Vec<_>>();
    let mut active = HashMap::new();
    assert!(ready_jobs(&store, &active, &mut 0).unwrap().is_empty());
    apply(
        &store,
        &mut active,
        Command::StartImmediately(target.id.clone()),
    )
    .unwrap();
    apply(
        &store,
        &mut active,
        Command::StartImmediately(target.id.clone()),
    )
    .unwrap();
    let ready = ready_jobs(&store, &active, &mut 0).unwrap();
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].1.id, target.id);
    assert_eq!(ready[0].1.downloaded, 1234);
    assert_eq!(ready[0].1.name, target.name);
    assert_eq!(
        store
            .queue_jobs(&queue.id)
            .unwrap()
            .into_iter()
            .map(|j| j.id)
            .collect::<Vec<_>>(),
        before_order
    );
    assert_eq!(
        store
            .queues()
            .unwrap()
            .into_iter()
            .find(|q| q.id == queue.id)
            .unwrap(),
        before_queue
    );
    let mut running = ready[0].1.clone();
    running.status = Status::Downloading;
    store.save(&running).unwrap();
    active.insert(
        target.id.clone(),
        Active {
            token: CancellationToken::new(),
            queue: queue.id.clone(),
            user_paused: false,
        },
    );
    assert!(ready_jobs(&store, &active, &mut 0).unwrap().is_empty());
    assert!(!active[&target.id].token.is_cancelled());
    apply(&store, &mut active, Command::PauseQueue(queue.id.clone())).unwrap();
    assert!(active[&target.id].token.is_cancelled());
    assert!(store.immediate_downloads().unwrap().is_empty());
}

#[test]
fn resume_commits_queue_activation_together_and_preserves_individual_pauses() {
    let (_root, store) = setup();
    let jobs = add(&store, MAIN_QUEUE, 2);
    for mut job in jobs.clone() {
        job.status = Status::Paused;
        store.save(&job).unwrap();
    }
    store.set_queue_running(MAIN_QUEUE, false).unwrap();
    let conn = store.connect().unwrap();
    conn.execute_batch("CREATE TRIGGER reject_start BEFORE UPDATE OF running ON download_queues BEGIN SELECT RAISE(ABORT,'Fixture queue save failure'); END;").unwrap();
    let mut active = HashMap::new();
    assert!(apply(&store, &mut active, Command::Resume(jobs[0].id.clone())).is_err());
    assert_eq!(
        store.get(&jobs[0].id).unwrap().unwrap().status,
        Status::Paused
    );
    assert!(!store.queues().unwrap()[0].running);
    conn.execute_batch("DROP TRIGGER reject_start;").unwrap();
    apply(&store, &mut active, Command::Resume(jobs[0].id.clone())).unwrap();
    assert_eq!(
        store.get(&jobs[0].id).unwrap().unwrap().status,
        Status::Queued
    );
    assert_eq!(
        store.get(&jobs[1].id).unwrap().unwrap().status,
        Status::Paused
    );
    assert!(store.queues().unwrap()[0].running);
}

#[test]
fn immediate_start_respects_global_slots_quotas_and_manual_pause() {
    let (_root, store) = setup();
    let jobs = add(&store, MAIN_QUEUE, 3);
    store.set_queue_limit(MAIN_QUEUE, 1).unwrap();
    let mut active = HashMap::from([(
        jobs[0].id.clone(),
        Active {
            token: CancellationToken::new(),
            queue: MAIN_QUEUE.into(),
            user_paused: false,
        },
    )]);
    let mut running = jobs[0].clone();
    running.status = Status::Downloading;
    store.save(&running).unwrap();
    apply(
        &store,
        &mut active,
        Command::StartImmediately(jobs[2].id.clone()),
    )
    .unwrap();
    let ready = ready_jobs(&store, &active, &mut 0).unwrap();
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].1.id, jobs[2].id);
    for i in 1..MAX_ACTIVE {
        active.insert(
            format!("busy-{i}"),
            Active {
                token: CancellationToken::new(),
                queue: "other".into(),
                user_paused: false,
            },
        );
    }
    assert!(ready_jobs(&store, &active, &mut 0).unwrap().is_empty());
    active.retain(|id, _| id == &jobs[0].id);
    store.connect().unwrap().execute("UPDATE data_quotas SET hourly_limit=1048576,hourly_used=1048576,hour_start=?1 WHERE id=1",[chrono::Utc::now().timestamp()]).unwrap();
    assert!(ready_jobs(&store, &active, &mut 0).unwrap().is_empty());
    store
        .connect()
        .unwrap()
        .execute("UPDATE data_quotas SET hourly_limit=NULL WHERE id=1", [])
        .unwrap();
    apply(&store, &mut active, Command::Pause(jobs[2].id.clone())).unwrap();
    assert!(store.immediate_downloads().unwrap().is_empty());
    assert_eq!(
        store.get(&jobs[2].id).unwrap().unwrap().status,
        Status::Paused
    );
    assert!(ready_jobs(&store, &active, &mut 0).unwrap().is_empty());
    apply(
        &store,
        &mut active,
        Command::StartImmediately(jobs[2].id.clone()),
    )
    .unwrap();
    store.recover().unwrap();
    assert!(store.immediate_downloads().unwrap().is_empty());
}

#[test]
fn queue_pause_rapid_start_and_manual_pause_keep_distinct_intent() {
    let (_temp, store) = setup();
    let jobs = add(&store, MAIN_QUEUE, 3);
    let mut active = HashMap::new();
    for job in &jobs[..2] {
        let mut running = job.clone();
        running.status = Status::Downloading;
        store.save(&running).unwrap();
        active.insert(
            job.id.clone(),
            Active {
                queue: MAIN_QUEUE.into(),
                token: CancellationToken::new(),
                user_paused: false,
            },
        );
    }
    apply(&store, &mut active, Command::Pause(jobs[1].id.clone())).unwrap();
    apply(&store, &mut active, Command::PauseQueue(MAIN_QUEUE.into())).unwrap();
    assert!(active.values().all(|w| w.token.is_cancelled()));
    apply(&store, &mut active, Command::StartQueue(MAIN_QUEUE.into())).unwrap();
    let ready = ready_jobs(&store, &active, &mut 0).unwrap();
    assert_eq!(
        ready.iter().map(|(_, j)| j.id.clone()).collect::<Vec<_>>(),
        vec![jobs[2].id.clone()]
    );
    for job in &jobs[..2] {
        let mut finished = job.clone();
        finished.status = Status::Paused;
        store.save(&finished).unwrap();
    }
    active.clear();
    let ready = ready_jobs(&store, &active, &mut 0).unwrap();
    assert!(ready.iter().any(|(_, j)| j.id == jobs[0].id));
    assert!(!ready.iter().any(|(_, j)| j.id == jobs[1].id));
    assert_eq!(
        store.get(&jobs[1].id).unwrap().unwrap().status,
        Status::Paused
    );
}

#[test]
fn reducing_limit_cancels_only_overflow_and_active_moves_are_rejected() {
    let (_temp, store) = setup();
    let jobs = add(&store, MAIN_QUEUE, 3);
    let mut active = HashMap::new();
    for job in &jobs {
        let mut running = job.clone();
        running.status = Status::Downloading;
        store.save(&running).unwrap();
        active.insert(
            job.id.clone(),
            Active {
                queue: MAIN_QUEUE.into(),
                token: CancellationToken::new(),
                user_paused: false,
            },
        );
    }
    apply(
        &store,
        &mut active,
        Command::SetQueueLimit(MAIN_QUEUE.into(), 1),
    )
    .unwrap();
    assert!(!active[&jobs[0].id].token.is_cancelled());
    assert!(active[&jobs[1].id].token.is_cancelled());
    assert!(active[&jobs[2].id].token.is_cancelled());
    assert!(ready_jobs(&store, &active, &mut 0).unwrap().is_empty());
    let other = store.create_queue("Other").unwrap();
    assert!(
        apply(
            &store,
            &mut active,
            Command::MoveJob(jobs[1].id.clone(), other.id.clone())
        )
        .is_err()
    );
    assert!(
        apply(
            &store,
            &mut active,
            Command::ReorderJob(jobs[1].id.clone(), true)
        )
        .is_err()
    );
    let mut finished = jobs[1].clone();
    finished.status = Status::Paused;
    store.save(&finished).unwrap();
    active.remove(&finished.id);
    apply(
        &store,
        &mut active,
        Command::MoveJob(finished.id.clone(), other.id.clone()),
    )
    .unwrap();
    assert_eq!(store.job_queue(&finished.id).unwrap(), Some(other.id));
    assert_eq!(
        store.get(&finished.id).unwrap().unwrap().status,
        Status::Queued
    );
}
#[test]
fn full_capacity_ticks_do_not_steal_another_queues_turn() {
    let (_temp, store) = setup();
    store.set_queue_limit(MAIN_QUEUE, MAX_ACTIVE).unwrap();
    let a = add(&store, MAIN_QUEUE, MAX_ACTIVE + 2);
    let other = store.create_queue("Other").unwrap();
    add(&store, &other.id, 10);
    store.set_queue_running(&other.id, true).unwrap();
    let mut active = HashMap::new();
    for job in &a[..MAX_ACTIVE - 1] {
        let mut running = job.clone();
        running.status = Status::Downloading;
        store.save(&running).unwrap();
        active.insert(
            job.id.clone(),
            Active {
                queue: MAIN_QUEUE.into(),
                token: CancellationToken::new(),
                user_paused: false,
            },
        );
    }
    let mut turn = 0;
    let mut first = ready_jobs(&store, &active, &mut turn).unwrap().remove(0);
    assert_eq!(first.0, MAIN_QUEUE);
    first.1.status = Status::Downloading;
    store.save(&first.1).unwrap();
    active.insert(
        first.1.id,
        Active {
            queue: first.0,
            token: CancellationToken::new(),
            user_paused: false,
        },
    );
    assert!(ready_jobs(&store, &active, &mut turn).unwrap().is_empty());
    active.remove(&a[0].id);
    let next = ready_jobs(&store, &active, &mut turn).unwrap();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].0, other.id);
}
#[test]
fn retry_wait_skips_slots_preserves_order_and_obeys_queue_controls() {
    let (_temp, store) = setup();
    store.set_queue_limit(MAIN_QUEUE, 1).unwrap();
    let mut jobs = add(&store, MAIN_QUEUE, 2);
    jobs[0].retry = retry::State {
        attempts: 2,
        next_at: Some(9000),
    };
    store.save(&jobs[0]).unwrap();
    let mut active = HashMap::new();
    let ready = ready_jobs_at(&store, &active, &mut 0, 8999).unwrap();
    assert_eq!(ready[0].1.id, jobs[1].id);
    let ready = ready_jobs_at(&store, &active, &mut 0, 9000).unwrap();
    assert_eq!(ready[0].1.id, jobs[0].id);
    apply(&store, &mut active, Command::PauseQueue(MAIN_QUEUE.into())).unwrap();
    assert!(
        ready_jobs_at(&store, &active, &mut 0, 10000)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store.get(&jobs[0].id).unwrap().unwrap().retry,
        jobs[0].retry
    );
    apply(&store, &mut active, Command::StartQueue(MAIN_QUEUE.into())).unwrap();
    apply(&store, &mut active, Command::PauseAll).unwrap();
    assert!(
        ready_jobs_at(&store, &active, &mut 0, 10000)
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .get(&jobs[0].id)
            .unwrap()
            .unwrap()
            .retry
            .next_at
            .is_none()
    );
    apply(&store, &mut active, Command::Resume(jobs[0].id.clone())).unwrap();
    assert!(store.get(&jobs[0].id).unwrap().unwrap().retry.is_default());
}
#[test]
fn pause_after_worker_saves_retry_still_wins_when_worker_is_joined() {
    let (_temp, store) = setup();
    let mut job = add(&store, MAIN_QUEUE, 1).remove(0);
    let mut active = HashMap::from([(
        job.id.clone(),
        Active {
            token: CancellationToken::new(),
            queue: MAIN_QUEUE.into(),
            user_paused: false,
        },
    )]);
    apply(&store, &mut active, Command::Pause(job.id.clone())).unwrap();
    // Model an outcome saved concurrently just before worker completion.
    job.retry = retry::State {
        attempts: 1,
        next_at: Some(9000),
    };
    store.save(&job).unwrap();
    let worker = active.remove(&job.id).unwrap();
    settle_worker(&store, &worker, &job.id).unwrap();
    let job = store.get(&job.id).unwrap().unwrap();
    assert_eq!(job.status, Status::Paused);
    assert!(job.retry.next_at.is_none());
    assert!(
        ready_jobs_at(&store, &active, &mut 0, 10000)
            .unwrap()
            .is_empty()
    );
}
#[test]
fn retry_move_remove_restart_and_schedule_do_not_leave_delayed_work() {
    let (_temp, store) = setup();
    let mut job = add(&store, MAIN_QUEUE, 1).remove(0);
    job.retry = retry::State {
        attempts: 3,
        next_at: Some(9000),
    };
    store.save(&job).unwrap();
    let mut active = HashMap::new();
    let other = store.create_queue("Later").unwrap();
    apply(
        &store,
        &mut active,
        Command::MoveJob(job.id.clone(), other.id.clone()),
    )
    .unwrap();
    assert_eq!(store.get(&job.id).unwrap().unwrap().retry, job.retry);
    let due = chrono::Utc::now().timestamp() + 30;
    apply(
        &store,
        &mut active,
        Command::ScheduleQueue(other.id.clone(), due),
    )
    .unwrap();
    assert!(
        ready_jobs_at(&store, &active, &mut 0, (due - 1) * 1000)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        ready_jobs_at(&store, &active, &mut 0, due * 1000).unwrap()[0]
            .1
            .id,
        job.id
    );
    apply(&store, &mut active, Command::Restart(job.id.clone())).unwrap();
    assert!(store.get(&job.id).unwrap().unwrap().retry.is_default());
    apply(&store, &mut active, Command::Remove(job.id)).unwrap();
    assert!(
        ready_jobs_at(&store, &active, &mut 0, due * 1000)
            .unwrap()
            .is_empty()
    );
}
