use super::*;
use crate::{paths::AppPaths, queue::MAIN_QUEUE};
use std::time::{Duration, Instant};

fn fixture() -> (tempfile::TempDir, Store, Project) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    store.set_queue_running(MAIN_QUEUE, false).unwrap();
    let project = store
        .create_grabber_project(
            "Manuals",
            &Settings {
                url: "https://example.test/".into(),
                extensions: "pdf".into(),
                depth: 1,
                page_limit: 10,
                queue: MAIN_QUEUE.into(),
                destination: Destination::Documents,
            },
        )
        .unwrap();
    (temp, store, project)
}

fn report(names: &[&str]) -> Report {
    Report {
        files: names
            .iter()
            .map(|name| Candidate {
                url: format!("https://example.test/{name}"),
                name: (*name).into(),
                filename: None,
                page: "https://example.test/".into(),
            })
            .collect(),
        pages_read: 1,
        ..Default::default()
    }
}

fn run(store: &Store, at: i64, result: Report) {
    let claim = store.claim_grabber(at).unwrap().expect("due scan");
    store.finish_grabber(&claim, Ok(result), at).unwrap();
}

#[test]
fn scheduled_discoveries_survive_restart_merge_and_only_queue_reviewed_files() {
    let (_temp, store, project) = fixture();
    store
        .set_grabber_schedule(&project, 100, Some(1), 99)
        .unwrap();
    assert!(store.claim_grabber(99).unwrap().is_none());
    run(
        &store,
        100,
        report(&["a.pdf#one", "a.pdf#two", "b.pdf?token=one"]),
    );
    assert!(store.list().unwrap().is_empty());
    let files = store.grabber_discoveries(&project.id).unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].file.url, "https://example.test/a.pdf");
    drop(store);
    let store = Store::open(AppPaths {
        data: _temp.path().join("data"),
        downloads: _temp.path().join("downloads"),
    })
    .unwrap();
    assert_eq!(
        store.grabber_discoveries(&project.id).unwrap()[0].id,
        files[0].id
    );
    // Another scan may finish while an old preview is open.
    run(&store, 3700, report(&["a.pdf", "b.pdf?token=one", "c.pdf"]));
    let jobs = store
        .review_grabber_files(
            &project.id,
            &[files[0].id.clone()],
            Some((MAIN_QUEUE, Destination::Documents)),
        )
        .unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(store.get(&jobs[0]).unwrap().unwrap().name, "a.pdf");
    let remaining = store.grabber_discoveries(&project.id).unwrap();
    assert_eq!(
        remaining
            .iter()
            .map(|d| d.file.name.as_str())
            .collect::<Vec<_>>(),
        ["b.pdf?token=one", "c.pdf"]
    );
    assert!(
        store
            .review_grabber_files(
                &project.id,
                &[files[0].id.clone()],
                Some((MAIN_QUEUE, Destination::Downloads))
            )
            .is_err()
    );
    assert_eq!(store.list().unwrap().len(), 1);
    store
        .review_grabber_files(&project.id, &[remaining[0].id.clone()], None)
        .unwrap();
    run(
        &store,
        7300,
        report(&["a.pdf", "b.pdf?token=one", "c.pdf", "b.pdf?token=two"]),
    );
    assert_eq!(store.grabber_discoveries(&project.id).unwrap().len(), 2);
    assert_eq!(store.list().unwrap().len(), 1);
}

#[test]
fn reviewing_is_atomic_on_invalid_queue_stale_selection_and_sql_failure() {
    let (_temp, store, project) = fixture();
    store.set_grabber_schedule(&project, 10, None, 9).unwrap();
    run(&store, 10, report(&["a.pdf", "b.pdf"]));
    let ids: Vec<_> = store
        .grabber_discoveries(&project.id)
        .unwrap()
        .into_iter()
        .map(|d| d.id)
        .collect();
    for (selection, queue) in [
        (ids.clone(), "missing"),
        (vec![ids[0].clone(), "stale".into()], MAIN_QUEUE),
    ] {
        assert!(
            store
                .review_grabber_files(
                    &project.id,
                    &selection,
                    Some((queue, Destination::Downloads))
                )
                .is_err()
        );
        assert!(store.list().unwrap().is_empty());
        assert_eq!(store.grabber_discoveries(&project.id).unwrap().len(), 2);
    }
    let conn = store.connect().unwrap();
    conn.execute_batch("CREATE TRIGGER fail_review BEFORE DELETE ON grabber_pending BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;").unwrap();
    assert!(
        store
            .review_grabber_files(
                &project.id,
                &ids,
                Some((MAIN_QUEUE, Destination::Downloads))
            )
            .is_err()
    );
    assert!(store.list().unwrap().is_empty());
    assert_eq!(store.grabber_discoveries(&project.id).unwrap().len(), 2);
    let receipts: i64 = conn
        .query_row("SELECT COUNT(*) FROM browser_batches", [], |r| r.get(0))
        .unwrap();
    assert_eq!(receipts, 0);
    conn.execute("DROP TRIGGER fail_review", []).unwrap();
    assert_eq!(
        store
            .review_grabber_files(
                &project.id,
                &ids,
                Some((MAIN_QUEUE, Destination::Downloads))
            )
            .unwrap()
            .len(),
        2
    );
    assert!(store.grabber_discoveries(&project.id).unwrap().is_empty());
}

#[test]
fn once_repeat_missed_runs_clock_changes_and_crash_recovery() {
    let (_temp, store, project) = fixture();
    for (at, hours) in [
        (99, None),
        (100, Some(0)),
        (100, Some(721)),
        (i64::MAX, None),
    ] {
        assert!(store.set_grabber_schedule(&project, at, hours, 99).is_err());
    }
    store
        .set_grabber_schedule(&project, 100, Some(1), 99)
        .unwrap();
    let claim = store.claim_grabber(100).unwrap().unwrap();
    let reopened = Store::open(store.paths.clone()).unwrap();
    assert!(reopened.claim_grabber(100).unwrap().is_none());
    reopened.recover_grabber_scans().unwrap();
    let recovered = reopened.claim_grabber(14_500).unwrap().unwrap();
    store
        .finish_grabber(&claim, Ok(report(&["stale.pdf"])), 100)
        .unwrap();
    assert!(store.grabber_discoveries(&project.id).unwrap().is_empty());
    reopened
        .finish_grabber(&recovered, Ok(report(&["new.pdf"])), 14_500)
        .unwrap();
    assert_eq!(store.grabber_schedules().unwrap()[0].next_at, Some(18_100));
    assert!(store.claim_grabber(14_500).unwrap().is_none());
    assert!(store.claim_grabber(99).unwrap().is_none());
    store
        .set_grabber_schedule(&project, 20_000, None, 19_000)
        .unwrap();
    run(&store, 21_000, report(&["new.pdf"]));
    let summary = store.grabber_schedules().unwrap().remove(0);
    assert!(!summary.enabled && summary.next_at.is_none());
    assert!(store.claim_grabber(30_000).unwrap().is_none());
}

#[test]
fn schedule_edits_project_edits_deletion_and_history_reset_reject_stale_results() {
    let (_temp, store, project) = fixture();
    store
        .set_grabber_schedule(&project, 100, Some(1), 99)
        .unwrap();
    let claim = store.claim_grabber(100).unwrap().unwrap();
    store.clear_grabber_schedule(&project).unwrap();
    store
        .finish_grabber(&claim, Ok(report(&["cancelled.pdf"])), 101)
        .unwrap();
    assert!(store.grabber_discoveries(&project.id).unwrap().is_empty());
    store
        .set_grabber_schedule(&project, 200, Some(1), 199)
        .unwrap();
    let claim = store.claim_grabber(200).unwrap().unwrap();
    let changed = store
        .update_grabber_project(&project, "Updated manuals", &project.settings)
        .unwrap();
    store
        .finish_grabber(&claim, Ok(report(&["old-settings.pdf"])), 201)
        .unwrap();
    store.release_grabber(&claim).unwrap();
    assert!(
        store
            .set_grabber_schedule(&project, 300, None, 299)
            .is_err()
    );
    run(&store, 201, report(&["one.pdf", "two.pdf"]));
    let pending = store.grabber_discoveries(&project.id).unwrap();
    store
        .review_grabber_files(&project.id, &[pending[0].id.clone()], None)
        .unwrap();
    store.reset_grabber_history(&changed).unwrap();
    run(&store, 3800, report(&["one.pdf", "two.pdf"]));
    assert_eq!(store.grabber_discoveries(&project.id).unwrap().len(), 2);
    let claim = store.claim_grabber(7400).unwrap().unwrap();
    store.delete_grabber_project(&changed).unwrap();
    store
        .finish_grabber(&claim, Ok(report(&["ghost.pdf"])), 7400)
        .unwrap();
    for table in ["grabber_schedules", "grabber_pending", "grabber_seen"] {
        let count: i64 = store
            .connect()
            .unwrap()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn failures_and_partial_reports_preserve_existing_discoveries() {
    let (_temp, store, project) = fixture();
    store
        .set_grabber_schedule(&project, 100, Some(1), 99)
        .unwrap();
    run(&store, 100, report(&["old.pdf"]));
    let claim = store.claim_grabber(3700).unwrap().unwrap();
    store
        .finish_grabber(&claim, Err(anyhow::anyhow!("fixture unavailable")), 3700)
        .unwrap();
    assert!(
        store.grabber_schedules().unwrap()[0]
            .message
            .contains("fixture unavailable")
    );
    assert_eq!(store.grabber_discoveries(&project.id).unwrap().len(), 1);
    run(
        &store,
        7300,
        Report {
            pages_failed: 1,
            stop: StopReason::TimeLimit,
            ..report(&["new.pdf"])
        },
    );
    assert_eq!(store.grabber_discoveries(&project.id).unwrap().len(), 2);
    assert!(
        store.grabber_schedules().unwrap()[0]
            .message
            .contains("partial results")
    );
    let claim = store.claim_grabber(10_900).unwrap().unwrap();
    store
        .finish_grabber(
            &claim,
            Ok(Report {
                stop: StopReason::Cancelled,
                ..report(&["cancelled.pdf"])
            }),
            10_900,
        )
        .unwrap();
    assert_eq!(store.grabber_discoveries(&project.id).unwrap().len(), 2);
    assert!(store.claim_grabber(10_900).unwrap().is_some());
}

#[test]
fn pending_capacity_does_not_lose_unseen_links_and_history_has_a_hard_bound() {
    let (_temp, store, project) = fixture();
    store
        .set_grabber_schedule(&project, 100, Some(1), 99)
        .unwrap();
    let names: Vec<_> = (0..MAX_PENDING + 2).map(|i| format!("{i}.pdf")).collect();
    let names: Vec<_> = names.iter().map(String::as_str).collect();
    run(&store, 100, report(&names));
    assert_eq!(
        store.grabber_discoveries(&project.id).unwrap().len(),
        MAX_PENDING
    );
    assert!(store.claim_grabber(3700).unwrap().is_none());
    let id = store.grabber_discoveries(&project.id).unwrap()[0]
        .id
        .clone();
    store
        .review_grabber_files(&project.id, &[id], None)
        .unwrap();
    run(&store, 3700, report(&names));
    let files = store.grabber_discoveries(&project.id).unwrap();
    assert_eq!(files.len(), MAX_PENDING);
    assert!(files.iter().any(|d| d.file.name == "1000.pdf"));
    let mut conn = store.connect().unwrap();
    let tx = conn.transaction().unwrap();
    for i in MAX_PENDING + 1..MAX_HISTORY {
        let hash = Sha256::digest(format!("remembered-{i}").as_bytes());
        tx.execute(
            "INSERT INTO grabber_seen VALUES(?1,?2)",
            params![project.id, &hash[..]],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    store
        .review_grabber_files(&project.id, &[files[0].id.clone()], None)
        .unwrap();
    run(&store, 7300, report(&["excess.pdf"]));
    let summary = store.grabber_schedules().unwrap().remove(0);
    assert!(!summary.enabled && summary.message.contains("10,000"));
}

#[test]
fn only_one_concurrent_caller_can_claim_the_same_occurrence() {
    let (_temp, store, project) = fixture();
    store.set_grabber_schedule(&project, 100, None, 99).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let store = store.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.claim_grabber(100).unwrap().is_some()
            })
        })
        .collect();
    assert_eq!(
        workers
            .into_iter()
            .map(|w| usize::from(w.join().unwrap()))
            .sum::<usize>(),
        1
    );
}

#[test]
fn accumulated_large_url_preview_fits_one_atomic_batch() {
    let (_temp, store, project) = fixture();
    store
        .set_grabber_schedule(&project, 100, Some(1), 99)
        .unwrap();
    let names: Vec<_> = (0..150)
        .map(|n| format!("{n}.pdf?token={}", "x".repeat(7000)))
        .collect();
    // Several individually bounded scans must not grow a preview past the
    // downstream queue batch's payload limit.
    for (i, chunk) in names.chunks(50).enumerate() {
        let mut report = report(&chunk.iter().map(String::as_str).collect::<Vec<_>>());
        for file in &mut report.files {
            file.name = "file.pdf".into();
        }
        run(&store, 100 + (i as i64) * 3600, report);
    }
    let files = store.grabber_discoveries(&project.id).unwrap();
    assert!(files.len() > 50 && files.len() < 100);
    let bytes: i64 = store
        .connect()
        .unwrap()
        .query_row(
            "SELECT SUM(length(CAST(data AS BLOB))) FROM grabber_pending",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(bytes <= MAX_PENDING_BYTES as i64);
    let ids: Vec<_> = files.iter().map(|d| d.id.clone()).collect();
    assert_eq!(
        store
            .review_grabber_files(
                &project.id,
                &ids,
                Some((MAIN_QUEUE, Destination::Downloads))
            )
            .unwrap()
            .len(),
        files.len()
    );
    assert!(store.grabber_discoveries(&project.id).unwrap().is_empty());
    run(&store, 10_900, report(&[names[149].as_str()]));
    assert_eq!(
        store.grabber_discoveries(&project.id).unwrap().len(),
        1,
        "unretained URLs must still be new"
    );
}

#[tokio::test]
async fn missing_proxy_login_finishes_the_scheduled_occurrence_instead_of_leaving_a_lease() {
    let (_temp, store, project) = fixture();
    store
        .set_grabber_schedule(&project, 100, Some(1), 99)
        .unwrap();
    let proxy = crate::proxy::Settings {
        mode: crate::proxy::Mode::Custom,
        endpoint: "http://127.0.0.1:9/".into(),
        credential: Some(uuid::Uuid::new_v4()),
        ..Default::default()
    };
    store
        .connect()
        .unwrap()
        .execute(
            "INSERT OR REPLACE INTO proxy_settings(id,data) VALUES (1,?1)",
            [serde_json::to_string(&proxy).unwrap()],
        )
        .unwrap();
    let mut controller = Controller::default();
    controller.tick(&store, 100).await.unwrap();
    assert!(controller.active.is_none());
    let schedule = &store.grabber_schedules().unwrap()[0];
    assert!(schedule.message.contains("proxy login is unavailable"));
    assert!(store.claim_grabber(100).unwrap().is_none());
    assert!(store.claim_grabber(3700).unwrap().is_some());
}

#[tokio::test]
async fn controller_scans_real_html_without_fetching_files_and_releases_cancelled_work() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (_temp, store, project) = fixture();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let settings = Settings {
        url: format!("http://{address}/"),
        depth: 0,
        ..project.settings.clone()
    };
    let project = store
        .update_grabber_project(&project, &project.name, &settings)
        .unwrap();
    store
        .set_grabber_schedule(&project, 100, Some(1), 99)
        .unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            request.push(stream.read_u8().await.unwrap());
        }
        let request = String::from_utf8(request).unwrap();
        assert!(request.starts_with("GET / HTTP/1.1"));
        assert!(!request.to_lowercase().contains("cookie:"));
        let html = "<a href='first.pdf'>File</a><a href='second.pdf'>File</a>";
        stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",html.len()).as_bytes()).await.unwrap();
        listener
    });
    let mut controller = Controller::default();
    let deadline = Instant::now() + Duration::from_secs(8);
    while store.grabber_discoveries(&project.id).unwrap().len() < 2 {
        controller.tick(&store, 100).await.unwrap();
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let listener = server.await.unwrap();
    assert!(store.list().unwrap().is_empty());
    assert!(
        tokio::time::timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_err(),
        "discovery must not request file bodies"
    );
    controller.tick(&store, 3700).await.unwrap();
    let (mut stalled, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
        .await
        .unwrap()
        .unwrap();
    let mut request = [0u8; 1024];
    assert!(stalled.read(&mut request).await.unwrap() > 0);
    store.clear_grabber_schedule(&project).unwrap();
    tokio::time::timeout(Duration::from_secs(2), controller.tick(&store, 3700))
        .await
        .unwrap()
        .unwrap();
    assert!(controller.active.is_none());
    assert_eq!(store.grabber_discoveries(&project.id).unwrap().len(), 2);
    store
        .set_grabber_schedule(&project, 7400, None, 7399)
        .unwrap();
    controller.tick(&store, 7400).await.unwrap();
    let (_stalled, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), controller.shutdown(&store))
        .await
        .unwrap()
        .unwrap();
    assert!(store.grabber_schedules().unwrap()[0].enabled);
    assert!(store.claim_grabber(7400).unwrap().is_some());
}
