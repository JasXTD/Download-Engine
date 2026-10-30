use super::*;
use crate::{paths::AppPaths, store::Store};

fn fixture() -> (tempfile::TempDir, Store) {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: tmp.path().join("data"),
        downloads: tmp.path().join("downloads"),
    })
    .unwrap();
    store.set_queue_running("main", false).unwrap();
    (tmp, store)
}
fn script() -> Script {
    Script {
        id: uuid::Uuid::new_v4().to_string(),
        revision: 0,
        name: "Test script".into(),
        language: Language::PowerShell,
        body: "Write-Output 'hello'".into(),
        enabled: false,
        trigger: Trigger::Manual,
        timeout_seconds: 30,
        change_files: false,
        extension: None,
    }
}
#[test]
fn legacy_definitions_keep_powershell_and_runs_snapshot_the_language() {
    let mut legacy = serde_json::to_value(script()).unwrap();
    legacy.as_object_mut().unwrap().remove("language");
    let mut definition: Script = serde_json::from_value(legacy).unwrap();
    assert_eq!(definition.language, Language::PowerShell);
    let (_tmp, store) = fixture();
    definition.language = Language::Lua;
    definition.body = "print('hello')".into();
    definition = store.save_script(&definition).unwrap();
    let run = store
        .run_script(&definition.id, &uuid::Uuid::new_v4().to_string())
        .unwrap();
    definition.language = Language::PowerShell;
    store.save_script(&definition).unwrap();
    assert_eq!(
        store.claim_script(false).unwrap().unwrap().run.language,
        Language::Lua
    );
    let mut old_run = serde_json::to_value(run).unwrap();
    old_run.as_object_mut().unwrap().remove("language");
    assert_eq!(
        serde_json::from_value::<Run>(old_run).unwrap().language,
        Language::PowerShell
    );
}
#[test]
fn save_snapshot_and_stop_are_persistent_and_revision_checked() {
    let (_tmp, s) = fixture();
    let mut definition = s.save_script(&script()).unwrap();
    assert_eq!(definition.revision, 1);
    let id = uuid::Uuid::new_v4().to_string();
    let run = s.run_script(&definition.id, &id).unwrap();
    assert_eq!(s.run_script(&definition.id, &id).unwrap().id, run.id);
    assert!(
        s.run_script(&definition.id, &uuid::Uuid::new_v4().to_string())
            .is_err()
    );
    assert!(s.remove_script(&definition.id).is_err());
    definition.body = "throw 'new version'".into();
    let updated = s.save_script(&definition).unwrap();
    assert!(s.save_script(&definition).is_err());
    let claim = s.claim_script(false).unwrap().unwrap();
    assert_eq!(claim.body, "Write-Output 'hello'");
    assert_eq!(claim.run.revision, 1);
    assert_eq!(updated.revision, 2);
    s.stop_script(&run.id).unwrap();
    assert!(s.script_stop_requested(&run.id).unwrap());
    s.finish_script(&run.id, "stopped", None, None).unwrap();
    let reopened = Store::open(s.paths.clone()).unwrap();
    assert_eq!(reopened.script_run(&run.id).unwrap().state, "stopped");
    reopened.remove_script(&definition.id).unwrap();
    assert_eq!(reopened.script_runs(None).unwrap().len(), 1);
}

#[test]
fn invalid_definitions_and_missing_queues_are_rejected() {
    let (_tmp, s) = fixture();
    for mutate in [
        |s: &mut Script| s.id = "../escape".into(),
        |s: &mut Script| s.name = "  ".into(),
        |s: &mut Script| s.body = " ".into(),
        |s: &mut Script| s.body = "a\0b".into(),
        |s: &mut Script| s.body = "a".repeat(MAX_SCRIPT_BYTES + 1),
        |s: &mut Script| s.timeout_seconds = 0,
        |s: &mut Script| s.timeout_seconds = 3601,
        |s: &mut Script| s.trigger = Trigger::Interval { minutes: 0 },
        |s: &mut Script| s.trigger = Trigger::Interval { minutes: 10081 },
        |s: &mut Script| {
            s.trigger = Trigger::QueueCompleted {
                queue: "missing".into(),
            }
        },
        |s: &mut Script| s.revision = 1,
    ] {
        let mut definition = script();
        mutate(&mut definition);
        assert!(s.save_script(&definition).is_err(), "{definition:?}");
    }
    assert!(s.scripts().unwrap().is_empty());
}

#[test]
fn cancelling_a_queued_run_prevents_claim_and_does_not_delete_history() {
    let (_tmp, s) = fixture();
    let definition = s.save_script(&script()).unwrap();
    let run = s
        .run_script(&definition.id, &uuid::Uuid::new_v4().to_string())
        .unwrap();
    s.stop_script(&run.id).unwrap();
    s.stop_script(&run.id).unwrap();
    assert!(s.claim_script(false).unwrap().is_none());
    assert_eq!(s.script_run(&run.id).unwrap().state, "stopped");
    assert!(
        s.run_script(&definition.id, &uuid::Uuid::new_v4().to_string())
            .is_ok()
    );
}

#[test]
fn failed_launch_is_retained_across_a_transient_log_write_failure() {
    let (_tmp, s) = fixture();
    let mut controller = Controller::default();
    controller.tick(&s, 0).unwrap();
    let definition = s.save_script(&script()).unwrap();
    let run = s
        .run_script(&definition.id, &uuid::Uuid::new_v4().to_string())
        .unwrap();
    let root = s.paths.data.join("script-runtime");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join(&run.id), "owned collision fixture").unwrap();
    controller.tick(&s, 0).unwrap();
    s.connect().unwrap().execute_batch("CREATE TRIGGER refuse_log BEFORE INSERT ON script_logs BEGIN SELECT RAISE(FAIL,'fixture busy'); END;").unwrap();
    assert!(controller.tick(&s, 1).is_err());
    assert_eq!(s.script_run(&run.id).unwrap().state, "running");
    s.connect()
        .unwrap()
        .execute_batch("DROP TRIGGER refuse_log")
        .unwrap();
    controller.tick(&s, 2).unwrap();
    assert_eq!(s.script_run(&run.id).unwrap().state, "failed");
    assert!(
        s.script_logs(&run.id).unwrap()[0]
            .text
            .contains("Cannot start script")
    );
    assert_eq!(
        std::fs::read_to_string(root.join(&run.id)).unwrap(),
        "owned collision fixture"
    );
    controller.shutdown(&s).unwrap();
}

#[test]
fn stopped_runs_obey_the_same_history_limit() {
    let (_tmp, s) = fixture();
    let definition = s.save_script(&script()).unwrap();
    for _ in 0..104 {
        let run = s
            .run_script(&definition.id, &uuid::Uuid::new_v4().to_string())
            .unwrap();
        s.stop_script(&run.id).unwrap();
    }
    assert_eq!(s.script_runs(None).unwrap().len(), 100);
}

#[test]
fn recovery_cleans_known_temporary_files_but_preserves_unexpected_files() {
    let (_tmp, s) = fixture();
    let root = s
        .paths
        .data
        .join("script-runtime")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("script.ps1"), "old source").unwrap();
    std::fs::write(root.join("keep.bin"), b"keep").unwrap();
    let mut controller = Controller::default();
    controller.tick(&s, 0).unwrap();
    assert!(!root.join("script.ps1").exists());
    assert_eq!(std::fs::read(root.join("keep.bin")).unwrap(), b"keep");
    controller.shutdown(&s).unwrap();
}
#[test]
fn timed_triggers_coalesce_and_never_overlap() {
    let (_tmp, s) = fixture();
    let mut definition = script();
    definition.enabled = true;
    definition.trigger = Trigger::Interval { minutes: 1 };
    let definition = s.save_script(&definition).unwrap();
    let now = chrono::Utc::now().timestamp();
    s.schedule_scripts(now, false).unwrap();
    assert!(s.script_runs(None).unwrap().is_empty());
    s.schedule_scripts(now + 600, false).unwrap();
    assert_eq!(s.script_runs(None).unwrap().len(), 1);
    s.schedule_scripts(now + 6000, false).unwrap();
    assert_eq!(s.script_runs(None).unwrap().len(), 1);
    let run = s.claim_script(false).unwrap().unwrap();
    s.finish_script(&run.run.id, "succeeded", None, Some(0))
        .unwrap();
    s.schedule_scripts(now + 6000, false).unwrap();
    assert_eq!(s.script_runs(None).unwrap().len(), 2);
    let active = s.claim_script(false).unwrap().unwrap();
    s.finish_script(&active.run.id, "succeeded", None, Some(0))
        .unwrap();
    s.schedule_scripts(now + 6001, false).unwrap();
    assert_eq!(s.script_runs(Some(&definition.id)).unwrap().len(), 2);
}
fn completed_job(s: &Store, queue: &str) -> crate::model::Job {
    let job = s
        .new_download(
            "http://127.0.0.1/fixture.bin",
            None,
            None,
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
    let mut job = s.enqueue_batch(&[job], queue).unwrap().remove(0);
    job.status = crate::model::Status::Complete;
    job.saved_path = Some(s.paths.downloads.join(&job.name));
    s.save(&job).unwrap();
    job
}
#[test]
fn completion_trigger_filters_queue_carries_context_and_advances_once() {
    let (_tmp, s) = fixture();
    let other = s.create_queue("Other").unwrap();
    completed_job(&s, "main");
    let mut definition = script();
    definition.enabled = true;
    definition.trigger = Trigger::DownloadCompleted {
        queue: Some("main".into()),
    };
    s.save_script(&definition).unwrap();
    let now = chrono::Utc::now().timestamp();
    completed_job(&s, &other.id);
    s.schedule_scripts(now, false).unwrap();
    assert!(s.script_runs(None).unwrap().is_empty());
    let job = completed_job(&s, "main");
    s.schedule_scripts(now, false).unwrap();
    let run = s.claim_script(false).unwrap().unwrap();
    assert_eq!(run.run.context["downloads"][0]["id"], job.id);
    assert_eq!(
        run.run.context["downloads"][0]["saved_path"],
        serde_json::to_value(&job.saved_path).unwrap()
    );
    assert_eq!(run.run.context["downloads"].as_array().unwrap().len(), 1);
    s.finish_script(&run.run.id, "succeeded", None, Some(0))
        .unwrap();
    s.schedule_scripts(now, false).unwrap();
    assert_eq!(s.script_runs(None).unwrap().len(), 1);
}

#[test]
fn completion_backlog_batches_without_duplicates_and_marks_expired_events() {
    let (_tmp, s) = fixture();
    let mut definition = script();
    definition.enabled = true;
    definition.trigger = Trigger::DownloadCompleted { queue: None };
    let definition = s.save_script(&definition).unwrap();
    let mut connection = s.connect().unwrap();
    let tx = connection.transaction().unwrap();
    for _ in 0..205 {
        tx.execute(
            "INSERT INTO script_events(job_id,queue_id) VALUES(?1,'main')",
            [uuid::Uuid::new_v4().to_string()],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    let mut ids = std::collections::HashSet::new();
    for expected in [100, 100, 5] {
        s.schedule_scripts(0, false).unwrap();
        let claim = s.claim_script(false).unwrap().unwrap();
        let downloads = claim.run.context["downloads"].as_array().unwrap();
        assert_eq!(downloads.len(), expected);
        for job in downloads {
            assert!(ids.insert(job["id"].as_str().unwrap().to_string()));
        }
        assert_eq!(claim.run.context["events_expired"], false);
        s.finish_script(&claim.run.id, "succeeded", None, Some(0))
            .unwrap();
    }
    s.schedule_scripts(0, false).unwrap();
    assert!(s.claim_script(false).unwrap().is_none());
    connection.execute("DELETE FROM script_events", []).unwrap();
    connection
        .execute(
            "INSERT INTO script_events(seq,job_id,queue_id) VALUES(30000,?1,'main')",
            [uuid::Uuid::new_v4().to_string()],
        )
        .unwrap();
    s.schedule_scripts(0, false).unwrap();
    let claim = s.claim_script(false).unwrap().unwrap();
    assert_eq!(claim.run.context["events_expired"], true);
    assert_eq!(claim.run.script_id, definition.id);
}
#[test]
fn queue_trigger_waits_for_every_file_and_can_fire_for_a_new_cycle() {
    let (_tmp, s) = fixture();
    let mut definition = script();
    definition.enabled = true;
    definition.trigger = Trigger::QueueCompleted {
        queue: "main".into(),
    };
    s.save_script(&definition).unwrap();
    let pending = s
        .new_download(
            "http://127.0.0.1/pending.bin",
            None,
            None,
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
    let mut pending = s.enqueue(&pending).unwrap();
    completed_job(&s, "main");
    s.schedule_scripts(100, false).unwrap();
    assert!(s.script_runs(None).unwrap().is_empty());
    pending.status = crate::model::Status::Complete;
    s.save(&pending).unwrap();
    s.schedule_scripts(101, false).unwrap();
    let run = s.claim_script(false).unwrap().unwrap();
    assert_eq!(run.run.context["queue"], "main");
    assert_eq!(run.run.context["downloads"], serde_json::json!([]));
    s.finish_script(&run.run.id, "succeeded", None, Some(0))
        .unwrap();
    s.schedule_scripts(102, false).unwrap();
    assert_eq!(s.script_runs(None).unwrap().len(), 1);
    completed_job(&s, "main");
    s.schedule_scripts(103, false).unwrap();
    assert_eq!(s.script_runs(None).unwrap().len(), 2);
}
#[test]
fn recovery_marks_pending_runs_interrupted_and_skips_old_events() {
    let (_tmp, s) = fixture();
    let mut definition = script();
    definition.enabled = true;
    definition.trigger = Trigger::DownloadCompleted { queue: None };
    let definition = s.save_script(&definition).unwrap();
    let run = s
        .run_script(&definition.id, &uuid::Uuid::new_v4().to_string())
        .unwrap();
    s.claim_script(false).unwrap();
    completed_job(&s, "main");
    s.recover_scripts().unwrap();
    assert_eq!(s.script_run(&run.id).unwrap().state, "interrupted");
    s.schedule_scripts(100, false).unwrap();
    assert!(s.claim_script(false).unwrap().is_none());
    completed_job(&s, "main");
    s.schedule_scripts(101, false).unwrap();
    assert!(s.claim_script(false).unwrap().is_some());
}
#[test]
fn startup_trigger_runs_only_on_startup_and_disabled_scripts_are_manual_only() {
    let (_tmp, s) = fixture();
    let mut definition = script();
    definition.trigger = Trigger::Startup;
    let mut definition = s.save_script(&definition).unwrap();
    s.schedule_scripts(100, true).unwrap();
    assert!(s.script_runs(None).unwrap().is_empty());
    definition.enabled = true;
    s.save_script(&definition).unwrap();
    s.schedule_scripts(100, false).unwrap();
    assert!(s.script_runs(None).unwrap().is_empty());
    s.schedule_scripts(101, true).unwrap();
    let run = s.claim_script(false).unwrap().unwrap();
    assert_eq!(run.run.trigger, "startup");
    s.finish_script(&run.run.id, "succeeded", None, Some(0))
        .unwrap();
    s.schedule_scripts(102, false).unwrap();
    assert_eq!(s.script_runs(None).unwrap().len(), 1);
}
#[test]
fn logs_bound_utf8_and_retention_cascades_without_removing_active_runs() {
    let (_tmp, s) = fixture();
    let definition = s.save_script(&script()).unwrap();
    let first = s
        .run_script(&definition.id, &uuid::Uuid::new_v4().to_string())
        .unwrap();
    s.append_script_log(&first.id, "info", &"🌍".repeat(MAX_LOG_BYTES))
        .unwrap();
    s.append_script_log(&first.id, "info", "ignored").unwrap();
    let logs = s.script_logs(&first.id).unwrap();
    assert_eq!(logs.len(), 2);
    assert_eq!(logs[0].text.len(), MAX_LOG_BYTES);
    assert_eq!(logs[1].level, "warn");
    s.finish_script(&first.id, "succeeded", None, Some(0))
        .unwrap();
    for _ in 0..101 {
        let run = s
            .run_script(&definition.id, &uuid::Uuid::new_v4().to_string())
            .unwrap();
        s.finish_script(&run.id, "succeeded", None, Some(0))
            .unwrap();
    }
    assert_eq!(s.script_runs(None).unwrap().len(), 100);
    assert!(s.script_run(&first.id).is_err());
    assert_eq!(
        s.connect()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM script_logs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[cfg(windows)]
fn wait_run(c: &mut Controller, s: &Store, id: &str, until: impl Fn(&Run, &[Log]) -> bool) -> Run {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(25);
    loop {
        c.tick(s, chrono::Utc::now().timestamp()).unwrap();
        let run = s.script_run(id).unwrap();
        let logs = s.script_logs(id).unwrap();
        if until(&run, &logs) {
            return run;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Timed out: {run:?}, {logs:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(35));
    }
}
#[cfg(windows)]
#[test]
fn real_powershell_outputs_unicode_errors_exit_codes_and_bounds_floods() {
    let (_tmp, s) = fixture();
    let mut controller = Controller::default();
    for (body, state, code, needle) in [
        (
            "Write-Output 'こんにちは 🌍'; Write-Warning 'heads up'",
            "succeeded",
            0,
            "こんにちは 🌍",
        ),
        (
            "throw 'intentional failure'",
            "failed",
            1,
            "intentional failure",
        ),
        ("exit 7", "failed", 7, ""),
        ("if (", "failed", 1, ""),
        // Scripts written before 2.0 use the old helper and variable names.
        (
            "if ((Get-Command Invoke-XDownloader).Definition -eq 'Invoke-DownloadEngine' -and $XDownloader -eq $DownloadEngine) { Write-Output 'legacy names ok' } else { exit 3 }",
            "succeeded",
            0,
            "legacy names ok",
        ),
        (
            "[Console]::Out.Write(('a' * 200000)); Write-Output 'end'",
            "succeeded",
            0,
            "Output truncated",
        ),
    ] {
        let mut definition = script();
        definition.body = body.into();
        let definition = s.save_script(&definition).unwrap();
        let run = s
            .run_script(&definition.id, &uuid::Uuid::new_v4().to_string())
            .unwrap();
        let run = wait_run(&mut controller, &s, &run.id, |r, _| !r.active());
        let logs = s.script_logs(&run.id).unwrap();
        assert_eq!(run.state, state, "{logs:?}");
        assert_eq!(run.exit_code, Some(code));
        let output = logs.iter().map(|l| l.text.as_str()).collect::<String>();
        assert!(output.contains(needle), "{output}");
        assert!(output.len() < MAX_LOG_BYTES + 200);
        assert!(!s.paths.data.join("script-runtime").join(&run.id).exists());
    }
    controller.shutdown(&s).unwrap();
}
#[cfg(windows)]
#[test]
fn real_powershell_stop_timeout_shutdown_and_concurrency_are_enforced() {
    let (_tmp, s) = fixture();
    let mut controller = Controller::default();
    let mut runs = Vec::new();
    for _ in 0..5 {
        let mut definition = script();
        definition.body = "Write-Output 'ready'; Start-Sleep 60".into();
        definition.timeout_seconds = 120;
        let definition = s.save_script(&definition).unwrap();
        runs.push(
            s.run_script(&definition.id, &uuid::Uuid::new_v4().to_string())
                .unwrap(),
        );
    }
    wait_run(&mut controller, &s, &runs[0].id, |_, logs| {
        logs.iter().any(|l| l.text.contains("ready"))
    });
    assert_eq!(
        s.script_runs(None)
            .unwrap()
            .iter()
            .filter(|r| r.state == "running")
            .count(),
        4
    );
    assert_eq!(s.script_run(&runs[4].id).unwrap().state, "queued");
    s.stop_script(&runs[0].id).unwrap();
    assert_eq!(
        wait_run(&mut controller, &s, &runs[0].id, |r, _| !r.active()).state,
        "stopped"
    );
    controller.shutdown(&s).unwrap();
    assert!(s.script_runs(None).unwrap().iter().all(|r| !r.active()));
    assert_eq!(
        std::fs::read_dir(s.paths.data.join("script-runtime"))
            .unwrap()
            .count(),
        0
    );
    let mut definition = script();
    definition.body = "Start-Sleep 60".into();
    definition.timeout_seconds = 1;
    let definition = s.save_script(&definition).unwrap();
    let run = s
        .run_script(&definition.id, &uuid::Uuid::new_v4().to_string())
        .unwrap();
    let mut controller = Controller::default();
    assert_eq!(
        wait_run(&mut controller, &s, &run.id, |r, _| !r.active()).state,
        "timed_out"
    );
    controller.shutdown(&s).unwrap();
}

fn reader(pages: &[&str]) -> Script {
    Script {
        language: Language::Lua,
        body: READER_EXAMPLE.into(),
        enabled: true,
        trigger: Trigger::SiteReader {
            pages: pages.iter().map(|p| p.to_string()).collect(),
        },
        ..script()
    }
}

#[test]
fn site_readers_are_lua_only_with_valid_pages_and_never_scheduled() {
    let (_tmp, s) = fixture();
    let mut powershell = reader(&["example.com/watch/*"]);
    powershell.language = Language::PowerShell;
    assert!(
        s.save_script(&powershell)
            .unwrap_err()
            .to_string()
            .contains("Lua")
    );
    for pages in [
        &[][..],
        &["Example.com/*"],
        &["localhost/*"],
        &["example.com/*", "example.com/*"],
    ] {
        assert!(s.save_script(&reader(pages)).is_err(), "{pages:?}");
    }
    let saved = s.save_script(&reader(&["example.com/watch/*"])).unwrap();
    // Readers run only when a page is opened or tested, never on a schedule.
    s.schedule_scripts(chrono::Utc::now().timestamp(), true)
        .unwrap();
    assert!(s.script_runs(Some(&saved.id)).unwrap().is_empty());
    let automation = s.save_script(&script()).unwrap();
    assert!(
        s.run_site_reader(
            &automation.id,
            "https://example.com/watch/1",
            &uuid::Uuid::new_v4().to_string()
        )
        .is_err()
    );
    // Nor does a reader ever run as an automation script (CLI, MCP, Run script).
    let error = s
        .run_script(&saved.id, &uuid::Uuid::new_v4().to_string())
        .unwrap_err();
    assert!(error.to_string().contains("Test on URL"));
    assert!(s.script_runs(Some(&saved.id)).unwrap().is_empty());
}

#[test]
fn pages_choose_the_first_enabled_reader_and_built_in_sites_stay_built_in() {
    let (_tmp, s) = fixture();
    let first = s
        .save_script(&reader(&["*.example.com/v/*", "www.youtube.com/*"]))
        .unwrap();
    let second = s
        .save_script(&reader(&["m.example.com/v/*", "example.org/*"]))
        .unwrap();
    let found = |page: &str| s.site_reader_for(page).unwrap().map(|r| r.id);
    assert_eq!(found("https://m.example.com/v/1"), Some(first.id.clone()));
    assert_eq!(
        found("https://example.org/anything"),
        Some(second.id.clone())
    );
    assert_eq!(found("https://example.com/v/1"), None);
    assert_eq!(found("https://www.youtube.com/watch?v=YE7VzlLtp-4"), None);
    assert_eq!(found("not a url"), None);
    let mut off = first;
    off.enabled = false;
    s.save_script(&off).unwrap();
    assert_eq!(found("https://m.example.com/v/1"), Some(second.id));
}

#[test]
fn reader_runs_go_first_with_their_own_slot_and_keep_their_result() {
    let (tmp, s) = fixture();
    let automation = s.save_script(&script()).unwrap();
    let saved = s.save_script(&reader(&["example.com/watch/*"])).unwrap();
    s.run_script(&automation.id, &uuid::Uuid::new_v4().to_string())
        .unwrap();
    // The reserved slot takes readers only.
    assert!(s.claim_script(true).unwrap().is_none());
    let request = uuid::Uuid::new_v4().to_string();
    let run = s
        .run_site_reader(&saved.id, "https://example.com/watch/a1#t=3", &request)
        .unwrap();
    assert_eq!(
        s.run_site_reader(&saved.id, "https://example.com/watch/a1", &request)
            .unwrap()
            .id,
        run.id
    );
    assert_eq!(run.trigger, "site_reader");
    assert_eq!(run.context["url"], "https://example.com/watch/a1");
    let claimed = s.claim_script(false).unwrap().unwrap();
    assert_eq!(claimed.run.id, run.id, "a waiting reader starts first");
    assert_eq!(
        s.claim_script(false).unwrap().unwrap().run.script_id,
        automation.id
    );

    let folder = tmp.path().join("run");
    std::fs::create_dir(&folder).unwrap();
    assert!(runner::keep_result(&s, &run.id, &folder).is_err());
    std::fs::write(
        folder.join("result.json"),
        r#"{"items":[{"kind":"file","url":"https://example.com/list.m3u8"}]}"#,
    )
    .unwrap();
    assert!(runner::keep_result(&s, &run.id, &folder).is_err());
    std::fs::write(
        folder.join("result.json"),
        vec![b' '; reading::MAX_READING_BYTES + 1],
    )
    .unwrap();
    assert!(runner::keep_result(&s, &run.id, &folder).is_err());
    std::fs::write(
        folder.join("result.json"),
        r#"{"title":"Nature","items":[{"kind":"hls","url":"https://cdn.example.com/m.m3u8"}]}"#,
    )
    .unwrap();
    runner::keep_result(&s, &run.id, &folder).unwrap();
    let found = s.script_result(&run.id).unwrap().unwrap();
    assert_eq!(found.title.as_deref(), Some("Nature"));
    assert!(
        s.script_result(&uuid::Uuid::new_v4().to_string())
            .unwrap()
            .is_none()
    );
}

/// Plays the scheduler's part: claim the reader run, then finish it.
async fn finish_reader(s: Store, outcome: &'static str) {
    let claim = loop {
        if let Some(claim) = s.claim_script(true).unwrap() {
            break claim;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };
    let id = claim.run.id;
    match outcome {
        "found" => {
            s.save_script_result(&id, &Reading::from_value(serde_json::json!({"items":[{"kind":"dash","url":"https://cdn.example.com/a.mpd"}]})).unwrap()).unwrap();
            s.finish_script(&id, "succeeded", None, Some(0)).unwrap();
        }
        "failed" => {
            s.append_script_log(
                &id,
                "error",
                "runtime error: reader.lua:4: Not a video page\nstack traceback:\n  [C]: in ?\n",
            )
            .unwrap();
            s.finish_script(&id, "failed", Some("Script exited with 1".into()), Some(1))
                .unwrap();
        }
        _ => {}
    }
}

#[tokio::test]
async fn reading_a_page_waits_for_the_reader_and_reports_its_own_error() {
    let (_tmp, s) = fixture();
    let saved = s.save_script(&reader(&["example.com/watch/*"])).unwrap();
    let page = "https://example.com/watch/1";
    let cancel = tokio_util::sync::CancellationToken::new();
    let (found, ()) = tokio::join!(
        read_page(&s, &saved.id, page, &cancel),
        finish_reader(s.clone(), "found")
    );
    assert_eq!(
        found.unwrap().single_stream().unwrap().url,
        "https://cdn.example.com/a.mpd"
    );
    let (failed, ()) = tokio::join!(
        read_page(&s, &saved.id, page, &cancel),
        finish_reader(s.clone(), "failed")
    );
    assert_eq!(
        failed.unwrap_err().to_string(),
        "Test script: runtime error: reader.lua:4: Not a video page"
    );
    // Cancelling asks the scheduler to stop the running reader.
    let stop = tokio_util::sync::CancellationToken::new();
    let reading = read_page(&s, &saved.id, page, &stop);
    let cancelling = async {
        finish_reader(s.clone(), "running").await;
        stop.cancel();
    };
    let (cancelled, ()) = tokio::join!(reading, cancelling);
    assert!(cancelled.unwrap_err().to_string().contains("cancelled"));
    let latest = s.script_runs(Some(&saved.id)).unwrap().remove(0);
    assert!(s.script_stop_requested(&latest.id).unwrap());
}

fn failed_job(s: &Store, queue: &str, attempts: u8) -> crate::model::Job {
    let job = s
        .new_download(
            "http://127.0.0.1/secret-token/fixture.bin",
            None,
            None,
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
    let mut job = s.enqueue_batch(&[job], queue).unwrap().remove(0);
    // An automatic retry keeps the download queued: no failure yet.
    job.status = crate::model::Status::Queued;
    job.retry.attempts = attempts;
    job.retry.next_at = Some(1);
    job.error = Some("Connection timed out".into());
    s.save(&job).unwrap();
    job.status = crate::model::Status::Failed;
    job.retry.next_at = None;
    s.save(&job).unwrap();
    job
}
fn event_script(s: &Store, trigger: Trigger) -> Script {
    s.save_script(&Script {
        enabled: true,
        trigger,
        ..script()
    })
    .unwrap()
}

#[test]
fn failure_trigger_fires_once_after_automatic_retries_and_never_counts_as_completion() {
    let (_tmp, s) = fixture();
    let failed = event_script(&s, Trigger::DownloadFailed { queue: None });
    let completed = event_script(&s, Trigger::DownloadCompleted { queue: None });
    let job = failed_job(&s, "main", 3);
    s.schedule_scripts(100, false).unwrap();
    let runs = s.script_runs(None).unwrap();
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0].script_id, failed.id);
    assert_eq!(runs[0].trigger, "download_failed");
    assert_eq!(
        runs[0].context["downloads"],
        serde_json::json!([{"id":job.id,"queue":"main","name":job.name,"error":"Connection timed out","attempts":4}])
    );
    assert!(!runs[0].context.to_string().contains("secret-token"));
    // Saving the failed download again is not a new failure.
    s.save(&job).unwrap();
    s.finish_script(&runs[0].id, "succeeded", None, Some(0))
        .unwrap();
    s.schedule_scripts(101, false).unwrap();
    assert_eq!(s.script_runs(None).unwrap().len(), 1);
    // A completion still reaches only the completion script.
    completed_job(&s, "main");
    s.schedule_scripts(102, false).unwrap();
    let runs = s.script_runs(None).unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].script_id, completed.id);
}

#[test]
fn queue_completed_ignores_failures_and_failure_scripts_filter_their_queue() {
    let (_tmp, s) = fixture();
    let other = s.create_queue("Other").unwrap();
    completed_job(&s, "main");
    let queue = event_script(
        &s,
        Trigger::QueueCompleted {
            queue: "main".into(),
        },
    );
    let filtered = event_script(
        &s,
        Trigger::DownloadFailed {
            queue: Some(other.id.clone()),
        },
    );
    // A failure that leaves only finished files behind is not a completion.
    let failed = failed_job(&s, "main", 0);
    s.remove(&failed.id).unwrap();
    s.schedule_scripts(100, false).unwrap();
    assert!(s.script_runs(None).unwrap().is_empty());
    let job = failed_job(&s, &other.id, 0);
    s.schedule_scripts(101, false).unwrap();
    let runs = s.script_runs(None).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].script_id, filtered.id);
    assert_eq!(runs[0].context["downloads"][0]["id"], job.id);
    assert_eq!(runs[0].context["downloads"][0]["attempts"], 1);
    completed_job(&s, "main");
    s.schedule_scripts(102, false).unwrap();
    assert_eq!(s.script_runs(Some(&queue.id)).unwrap().len(), 1);
}

#[test]
fn sample_runs_match_the_real_event_and_check_the_chosen_downloads() {
    let (_tmp, s) = fixture();
    let failed = event_script(&s, Trigger::DownloadFailed { queue: None });
    let job = failed_job(&s, "main", 2);
    s.schedule_scripts(100, false).unwrap();
    let real = s.claim_script(false).unwrap().unwrap().run;
    s.finish_script(&real.id, "succeeded", None, Some(0))
        .unwrap();
    let request = uuid::Uuid::new_v4().to_string();
    let sample = s
        .run_script_sample(&failed.id, &request, std::slice::from_ref(&job.id))
        .unwrap();
    assert_eq!(sample.trigger, "download_failed");
    assert_eq!(sample.context["sample"], true);
    assert_eq!(sample.context["downloads"], real.context["downloads"]);
    assert_eq!(sample.context["queue"], real.context["queue"]);
    // Retrying a request returns the same run.
    assert_eq!(
        s.run_script_sample(&failed.id, &request, std::slice::from_ref(&job.id))
            .unwrap()
            .id,
        sample.id
    );
    s.finish_script(&sample.id, "succeeded", None, Some(0))
        .unwrap();
    let finished = completed_job(&s, "main");
    let missing = uuid::Uuid::new_v4().to_string();
    for (downloads, needle) in [
        (vec![], "Choose 1 to 100"),
        (vec![finished.id.clone()], "Failed"),
        (vec![missing], "no longer exists"),
        (vec![job.id.clone(), job.id.clone()], "once"),
        (vec![job.id.clone(); 101], "Choose 1 to 100"),
    ] {
        let error = s
            .run_script_sample(&failed.id, &uuid::Uuid::new_v4().to_string(), &downloads)
            .unwrap_err();
        assert!(error.to_string().contains(needle), "{error}");
    }
    let completed = event_script(&s, Trigger::DownloadCompleted { queue: None });
    let run = s
        .run_script_sample(
            &completed.id,
            &uuid::Uuid::new_v4().to_string(),
            std::slice::from_ref(&finished.id),
        )
        .unwrap();
    assert_eq!(
        run.context["downloads"][0]["saved_path"],
        serde_json::to_value(&finished.saved_path).unwrap()
    );
    let queue = event_script(
        &s,
        Trigger::QueueCompleted {
            queue: "main".into(),
        },
    );
    let run = s
        .run_script_sample(&queue.id, &uuid::Uuid::new_v4().to_string(), &[])
        .unwrap();
    assert_eq!(run.context["downloads"], serde_json::json!([]));
    assert_eq!(run.context["queue"], "main");
    assert!(
        s.run_script_sample(&queue.id, &uuid::Uuid::new_v4().to_string(), &[finished.id])
            .is_err()
    );
    let manual = s.save_script(&script()).unwrap();
    assert!(
        s.run_script_sample(&manual.id, &uuid::Uuid::new_v4().to_string(), &[])
            .is_err()
    );
    let (_, is_sample) = s.script_for_run(&run.id).unwrap();
    assert!(is_sample);
    let (_, is_sample) = s.script_for_run(&real.id).unwrap();
    assert!(!is_sample);
}

#[test]
fn stored_values_persist_per_script_and_enforce_their_limits() {
    let (_tmp, s) = fixture();
    let first = s.save_script(&script()).unwrap();
    let second = s.save_script(&script()).unwrap();
    let value = serde_json::json!({"hosts":["a.example","b.example"],"count":2});
    s.set_script_value(&first.id, "seen", &value).unwrap();
    s.set_script_value(&first.id, "été", &serde_json::json!("ok"))
        .unwrap();
    let reopened = Store::open(s.paths.clone()).unwrap();
    assert_eq!(
        reopened.script_value(&first.id, "seen").unwrap(),
        Some(value)
    );
    assert_eq!(
        reopened
            .script_values(&first.id)
            .unwrap()
            .iter()
            .map(|v| v.key.as_str())
            .collect::<Vec<_>>(),
        ["seen", "été"]
    );
    assert!(reopened.script_value(&second.id, "seen").unwrap().is_none());
    s.set_script_value(&first.id, "seen", &serde_json::Value::Null)
        .unwrap();
    assert!(s.script_value(&first.id, "seen").unwrap().is_none());
    for key in ["", &"k".repeat(MAX_STATE_KEY_CHARS + 1), "line\nbreak"] {
        assert!(
            s.set_script_value(&first.id, key, &serde_json::json!(1))
                .is_err(),
            "{key:?}"
        );
    }
    let large = serde_json::json!("x".repeat(MAX_STATE_VALUE_BYTES));
    assert!(s.set_script_value(&first.id, "large", &large).is_err());
    for i in 0..MAX_STATE_KEYS - 1 {
        s.set_script_value(&second.id, &format!("k{i}"), &serde_json::json!(i))
            .unwrap();
    }
    s.set_script_value(&second.id, "last", &serde_json::json!(true))
        .unwrap();
    // Replacing a value is allowed at the limit; a new key is not.
    s.set_script_value(&second.id, "last", &serde_json::json!(false))
        .unwrap();
    assert!(
        s.set_script_value(&second.id, "one more", &serde_json::json!(1))
            .is_err()
    );
    let third = s.save_script(&script()).unwrap();
    let chunk = serde_json::json!("y".repeat(MAX_STATE_VALUE_BYTES - 100));
    let mut stored = 0;
    while s
        .set_script_value(&third.id, &format!("c{stored}"), &chunk)
        .is_ok()
    {
        stored += 1;
    }
    // Each value is its JSON text (the string plus quotes) and its key.
    let each = MAX_STATE_VALUE_BYTES - 100 + 2 + 2;
    assert_eq!(stored, MAX_STATE_BYTES / each);
    assert_eq!(s.clear_script_values(&third.id).unwrap(), stored);
    assert!(
        s.set_script_value(
            &uuid::Uuid::new_v4().to_string(),
            "k",
            &serde_json::json!(1)
        )
        .is_err()
    );
    s.remove_script(&first.id).unwrap();
    assert!(s.script_values(&first.id).unwrap().is_empty());
}

#[test]
fn script_retries_stop_after_five_until_the_download_completes_or_is_removed() {
    let (_tmp, s) = fixture();
    let mut job = failed_job(&s, "main", 0);
    for _ in 0..MAX_SCRIPT_RETRIES {
        s.count_script_retry(&job.id).unwrap();
    }
    let error = s.count_script_retry(&job.id).unwrap_err();
    assert!(error.to_string().contains("already retried"), "{error}");
    s.uncount_script_retry(&job.id).unwrap();
    s.count_script_retry(&job.id).unwrap();
    assert!(s.count_script_retry(&job.id).is_err());
    job.status = crate::model::Status::Complete;
    s.save(&job).unwrap();
    s.count_script_retry(&job.id).unwrap();
    s.remove(&job.id).unwrap();
    let count: i64 = s
        .connect()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM script_retries", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn notifications_need_a_running_run_and_stop_at_five() {
    let (_tmp, s) = fixture();
    let definition = s.save_script(&script()).unwrap();
    let run = s
        .run_script(&definition.id, &uuid::Uuid::new_v4().to_string())
        .unwrap();
    assert!(s.count_script_notice(&run.id).is_err());
    s.claim_script(false).unwrap();
    for _ in 0..MAX_NOTICES {
        s.count_script_notice(&run.id).unwrap();
    }
    let error = s.count_script_notice(&run.id).unwrap_err();
    assert!(error.to_string().contains("at most 5"), "{error}");
    s.finish_script(&run.id, "succeeded", None, Some(0))
        .unwrap();
    assert!(
        s.count_script_notice(&run.id)
            .unwrap_err()
            .to_string()
            .contains("running")
    );
}

#[test]
fn readers_cannot_change_files_and_old_profiles_gain_the_new_columns() {
    let (_tmp, s) = fixture();
    let mut saved = reader(&["example.com/watch/*"]);
    saved.change_files = true;
    assert!(
        s.save_script(&saved)
            .unwrap_err()
            .to_string()
            .contains("cannot change")
    );
    // A 1.21 profile: events without a kind, runs without a notice count.
    let tmp = tempfile::tempdir().unwrap();
    let paths = crate::paths::AppPaths {
        data: tmp.path().join("data"),
        downloads: tmp.path().join("downloads"),
    };
    paths.prepare().unwrap();
    let legacy = rusqlite::Connection::open(paths.data.join("queue.sqlite3")).unwrap();
    legacy.execute_batch("CREATE TABLE script_events(seq INTEGER PRIMARY KEY AUTOINCREMENT,job_id TEXT NOT NULL,queue_id TEXT NOT NULL);
        INSERT INTO script_events(job_id,queue_id) VALUES('old','main');
        CREATE TABLE script_runs(id TEXT PRIMARY KEY,script_id TEXT NOT NULL,data TEXT NOT NULL,body TEXT NOT NULL,timeout INTEGER NOT NULL,state TEXT NOT NULL,stop_requested INTEGER NOT NULL DEFAULT 0,log_bytes INTEGER NOT NULL DEFAULT 0,truncated INTEGER NOT NULL DEFAULT 0);").unwrap();
    drop(legacy);
    let upgraded = Store::open(paths.clone()).unwrap();
    // Opening again, as every CLI call does, keeps the migration in place.
    Store::open(paths).unwrap();
    let conn = upgraded.connect().unwrap();
    let kind: String = conn
        .query_row(
            "SELECT kind FROM script_events WHERE job_id='old'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(kind, "completed");
    let notices: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('script_runs') WHERE name='notices'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(notices, 1);
}
