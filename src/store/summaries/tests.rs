use super::*;

#[test]
fn sql_pages_match_existing_summary_contract_for_filters_ties_and_empty_pages() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: root.path().join("data"),
        downloads: root.path().join("downloads"),
    })
    .unwrap();
    let other = store.create_queue("Other").unwrap();
    for n in 0..241 {
        let mut job = Job::new(
            "https://example.invalid/a.bin",
            Some(&format!("file-{n}.bin")),
            store.paths.downloads.clone(),
            format!("job-{n}"),
        )
        .unwrap();
        job.created = (n / 3) as u64;
        job.status = if n % 2 == 0 {
            Status::Paused
        } else {
            Status::Queued
        };
        store
            .enqueue_batch(&[job], if n % 3 == 0 { &other.id } else { "main" })
            .unwrap();
    }
    let all: Vec<_> = store
        .list()
        .unwrap()
        .iter()
        .map(|job| crate::control::job_summary(&store, job).unwrap())
        .collect();
    for queue in [None, Some("main"), Some(other.id.as_str()), Some("missing")] {
        for status in [None, Some("paused"), Some("queued"), Some("complete")] {
            let filtered: Vec<_> = all
                .iter()
                .filter(|j| {
                    queue.is_none_or(|q| j["queue"] == q) && status.is_none_or(|s| j["status"] == s)
                })
                .cloned()
                .collect();
            for offset in [0, 1, 17, 100, 250] {
                let page = store.download_summaries(queue, status, offset, 17).unwrap();
                let expected: Vec<_> = filtered.iter().skip(offset).take(17).cloned().collect();
                assert_eq!(page["items"], json!(expected));
                assert_eq!(page["total"], filtered.len());
                assert_eq!(
                    page["next_offset"],
                    json!(if offset + expected.len() < filtered.len() {
                        Some(offset + expected.len())
                    } else {
                        None
                    })
                );
            }
        }
    }
}

#[test]
fn small_page_does_not_decode_unrequested_job_snapshots() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: root.path().join("data"),
        downloads: root.path().join("downloads"),
    })
    .unwrap();
    let job = Job::new(
        "https://example.invalid/a.bin",
        None,
        store.paths.downloads.clone(),
        "newest".into(),
    )
    .unwrap();
    store.enqueue(&job).unwrap();
    // Valid JSON but deliberately not a valid Job. SQL count/filter can inspect
    // its status; full-list deserialization would fail even for a one-row page.
    store
        .connect()
        .unwrap()
        .execute(
            "INSERT INTO jobs VALUES(?1,'older',0,?2)",
            params![uuid::Uuid::new_v4().to_string(), r#"{"status":"paused"}"#],
        )
        .unwrap();
    assert!(store.list().is_err());
    let page = store.download_summaries(None, None, 0, 1).unwrap();
    assert_eq!(page["total"], 2);
    assert_eq!(page["items"][0]["id"], job.id);
    assert_eq!(page["next_offset"], 1);
    assert!(store.download_summaries(None, None, 1, 1).is_err());
}
