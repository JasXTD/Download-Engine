use super::*;
use crate::{model::Status, paths::AppPaths};

fn fixture() -> (tempfile::TempDir, Store) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    store.set_queue_running("main", false).unwrap();
    (temp, store)
}
fn job(store: &Store, url: &str) -> Job {
    store
        .new_download(url, None, None, uuid::Uuid::new_v4().to_string())
        .unwrap()
}
fn route(queue: &str) -> Rule {
    Rule {
        id: uuid::Uuid::new_v4().to_string(),
        name: "Archives".into(),
        enabled: true,
        workflow: Workflow::DownloadAdded {
            host: "example.com".into(),
            extension: "zip".into(),
            queue: queue.into(),
            bytes_per_second: Some(1024 * 1024),
        },
    }
}
fn chain(source: &str, start: &str) -> Rule {
    Rule {
        id: uuid::Uuid::new_v4().to_string(),
        name: "Next queue".into(),
        enabled: true,
        workflow: Workflow::QueueCompleted {
            source: source.into(),
            start: start.into(),
        },
    }
}
fn complete(store: &Store, j: &Job) {
    let mut j = j.clone();
    j.status = Status::Complete;
    store.save(&j).unwrap();
}

#[test]
fn routing_requires_both_conditions_and_uses_first_enabled_match() {
    let (_temp, s) = fixture();
    let work = s.create_queue("Work").unwrap();
    let later = s.create_queue("Later").unwrap();
    s.save_automation_rule(&route(&work.id)).unwrap();
    s.save_automation_rule(&route(&later.id)).unwrap();
    let good = job(&s, "https://example.com/FILE.ZIP?token=private");
    for url in [
        "https://example.com/file.mp4",
        "https://sub.example.com/file.zip",
        "https://example.com.evil.test/file.zip",
    ] {
        let j = s.enqueue(&job(&s, url)).unwrap();
        assert_eq!(s.job_queue(&j.id).unwrap().as_deref(), Some("main"));
    }
    s.enqueue(&good).unwrap();
    s.enqueue(&good).unwrap();
    assert_eq!(s.job_queue(&good.id).unwrap().unwrap(), work.id);
    assert_eq!(
        s.bandwidth_limits().unwrap().downloads[&good.id],
        1024 * 1024
    );
    let history = s.automation_history().unwrap();
    assert_eq!(history.len(), 1);
    assert!(!history[0].message.contains("private"));
    let mut rules = s.automation_rules().unwrap();
    rules[0].enabled = false;
    s.save_automation_rule(&rules[0]).unwrap();
    let second = s
        .enqueue(&job(&s, "https://example.com/second.zip"))
        .unwrap();
    assert_eq!(s.job_queue(&second.id).unwrap().unwrap(), later.id);
}

#[test]
fn routing_and_history_are_atomic_with_batch_and_browser_handoff() {
    let (_temp, s) = fixture();
    let q = s.create_queue("Work").unwrap();
    s.save_automation_rule(&route(&q.id)).unwrap();
    let a = job(&s, "https://example.com/a.zip");
    let b = job(&s, "https://example.com/unsupported.m3u8");
    assert!(s.enqueue_batch(&[a.clone(), b], "main").is_err());
    assert!(s.list().unwrap().is_empty());
    assert!(s.automation_history().unwrap().is_empty());
    s.prepare_capture(&a).unwrap();
    assert!(s.automation_history().unwrap().is_empty());
    s.commit_capture(&a.request_id).unwrap();
    s.commit_capture(&a.request_id).unwrap();
    assert_eq!(s.job_queue(&a.id).unwrap().unwrap(), q.id);
    assert_eq!(s.automation_history().unwrap().len(), 1);
}

#[test]
fn removed_destination_logs_once_without_losing_new_download() {
    let (_temp, s) = fixture();
    let q = s.create_queue("Work").unwrap();
    s.save_automation_rule(&route(&q.id)).unwrap();
    s.connect()
        .unwrap()
        .execute("DELETE FROM download_queues WHERE id=?1", [&q.id])
        .unwrap();
    let a = job(&s, "https://example.com/a.zip");
    s.enqueue(&a).unwrap();
    s.enqueue(&a).unwrap();
    assert_eq!(s.job_queue(&a.id).unwrap().as_deref(), Some("main"));
    let history = s.automation_history().unwrap();
    assert_eq!(history.len(), 1);
    assert!(!history[0].success);
    // A broken destination must not prevent the user from disabling the rule.
    let mut disabled = s.automation_rules().unwrap().remove(0);
    disabled.enabled = false;
    s.save_automation_rule(&disabled).unwrap();
    s.enqueue(&job(&s, "https://example.com/disabled.zip"))
        .unwrap();
    assert_eq!(s.automation_history().unwrap().len(), 1);
}

#[test]
fn completion_waits_for_every_file_and_survives_restart_without_replay() {
    let (_temp, s) = fixture();
    let target = s.create_queue("Next").unwrap();
    s.save_automation_rule(&chain("main", &target.id)).unwrap();
    s.run_queue_automations().unwrap();
    assert!(!s.queues().unwrap()[1].running);
    let a = s.enqueue(&job(&s, "https://example.com/a.bin")).unwrap();
    let mut b = s.enqueue(&job(&s, "https://example.com/b.bin")).unwrap();
    complete(&s, &a);
    b.status = Status::Failed;
    s.save(&b).unwrap();
    s.run_queue_automations().unwrap();
    assert!(s.automation_history().unwrap().is_empty());
    complete(&s, &b);
    s.run_queue_automations().unwrap();
    assert_eq!(s.automation_history().unwrap().len(), 1);
    s.set_queue_running(&target.id, false).unwrap();
    let reopened = Store::open(s.paths.clone()).unwrap();
    reopened.run_queue_automations().unwrap();
    assert!(
        !reopened
            .queues()
            .unwrap()
            .into_iter()
            .find(|q| q.id == target.id)
            .unwrap()
            .running
    );
    let c = s.enqueue(&job(&s, "https://example.com/c.bin")).unwrap();
    complete(&s, &c);
    reopened.run_queue_automations().unwrap();
    assert_eq!(reopened.automation_history().unwrap().len(), 2);
}

#[test]
fn creating_or_reenabling_rule_never_replays_past_completions() {
    let (_temp, s) = fixture();
    let q = s.create_queue("Next").unwrap();
    let a = s.enqueue(&job(&s, "https://example.com/a")).unwrap();
    complete(&s, &a);
    let mut rule = chain("main", &q.id);
    s.save_automation_rule(&rule).unwrap();
    s.run_queue_automations().unwrap();
    assert!(s.automation_history().unwrap().is_empty());
    rule.enabled = false;
    s.save_automation_rule(&rule).unwrap();
    let b = s.enqueue(&job(&s, "https://example.com/b")).unwrap();
    complete(&s, &b);
    s.run_queue_automations().unwrap();
    rule.enabled = true;
    s.save_automation_rule(&rule).unwrap();
    s.run_queue_automations().unwrap();
    assert!(s.automation_history().unwrap().is_empty());
}

#[test]
fn invalid_rules_are_rejected_and_history_is_bounded() {
    let (_temp, s) = fixture();
    for host in [
        "https://example.com",
        "example.com/path",
        "example.com:443",
        "user@example.com",
        "example.com?token=x",
    ] {
        let mut r = route("main");
        if let Workflow::DownloadAdded { host: h, .. } = &mut r.workflow {
            *h = host.into();
        }
        assert!(s.save_automation_rule(&r).is_err(), "{host}");
    }
    assert!(s.save_automation_rule(&chain("main", "main")).is_err());
    let r = route("main");
    s.save_automation_rule(&r).unwrap();
    let mut conn = s.connect().unwrap();
    let tx = conn.transaction().unwrap();
    for _ in 0..510 {
        record(&tx, &r, true, "fixture").unwrap();
    }
    tx.commit().unwrap();
    assert_eq!(s.automation_history().unwrap().len(), 500);
}

#[test]
fn queue_chains_cannot_cycle_including_when_a_disabled_rule_is_reenabled() {
    let (_temp, s) = fixture();
    let a = s.create_queue("A").unwrap();
    let b = s.create_sync_queue("B").unwrap();
    s.save_automation_rule(&chain("main", &a.id)).unwrap();
    s.save_automation_rule(&chain(&a.id, &b.id)).unwrap();
    let mut back = chain(&b.id, "main");
    assert!(
        s.save_automation_rule(&back)
            .unwrap_err()
            .to_string()
            .contains("loop")
    );
    back.enabled = false;
    s.save_automation_rule(&back).unwrap();
    back.enabled = true;
    assert!(s.save_automation_rule(&back).is_err());
    assert_eq!(s.automation_rules().unwrap().len(), 3);
    assert!(!s.automation_rules().unwrap()[2].enabled);
}
