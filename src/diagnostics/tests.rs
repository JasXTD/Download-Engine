use super::*;

fn fixture() -> (tempfile::TempDir, AppPaths) {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    };
    (temp, paths)
}
fn records(path: &Path) -> Vec<Record> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn secrets_never_enter_records_or_exports_and_error_codes_survive() {
    let (temp, paths) = fixture();
    let logger = Logger::new(&paths, Component::Desktop).unwrap();
    for secret in [
        "https://alice:secret@example.com/private.zip?token=HUSH",
        r"Cannot open C:\Users\Alice\Private report.pdf",
        "Authorization: Bearer HUSH\r\nCookie: session=HUSH",
        r"\\server\private folder\file.txt",
        "password=HUSH api_key=HUSH encoded=%73%65%63%72%65%74",
        "private-filename-with-no-path.pdf",
    ] {
        logger
            .write(
                Level::Error,
                Event::UiError,
                Details {
                    failure: Some(Failure::message(secret)),
                    ..Default::default()
                },
                false,
            )
            .unwrap();
    }
    let error =
        anyhow::Error::new(io::Error::from_raw_os_error(2)).context("HUSH: private context");
    logger
        .write(
            Level::Error,
            Event::StartupFailed,
            Details {
                failure: Some(Failure::error(&error)),
                ..Default::default()
            },
            false,
        )
        .unwrap();
    let entries = records(&log_path(&logger.root, Component::Desktop, 0));
    assert_eq!(
        entries
            .last()
            .unwrap()
            .details
            .failure
            .as_ref()
            .unwrap()
            .code,
        Some(2)
    );
    let output = export(&paths, temp.path()).unwrap();
    assert_eq!(
        Failure::message("Access refused (HTTP 403). SECRET").http_status,
        Some(403)
    );
    assert_eq!(Failure::message("HTTP 403999 SECRET").http_status, None);
    for text in [
        fs::read_to_string(log_path(&logger.root, Component::Desktop, 0)).unwrap(),
        fs::read_to_string(output).unwrap(),
    ] {
        for secret in [
            "alice",
            "Alice",
            "example.com",
            "HUSH",
            "Private report",
            "private folder",
            "private-filename",
            "%73",
        ] {
            assert!(!text.contains(secret), "Leaked {secret}");
        }
    }
}

#[test]
fn rotation_is_bounded_and_keeps_the_newest_complete_records() {
    let (_temp, paths) = fixture();
    let mut logger = Logger::new(&paths, Component::NativeHost).unwrap();
    logger.limit = 1024;
    for sequence in 0..100 {
        logger
            .write(
                Level::Info,
                Event::NativeRequestSucceeded,
                Details {
                    request: Some(sequence),
                    ..Default::default()
                },
                false,
            )
            .unwrap();
    }
    let mut sequences = Vec::new();
    for index in (0..=BACKUPS).rev() {
        let path = log_path(&logger.root, Component::NativeHost, index);
        assert!(fs::metadata(&path).unwrap().len() <= logger.limit);
        sequences.extend(records(&path).iter().map(|r| r.details.request.unwrap()));
    }
    assert_eq!(sequences.last(), Some(&99));
    assert!(sequences.windows(2).all(|pair| pair[1] == pair[0] + 1));
    assert!(!log_path(&logger.root, Component::NativeHost, BACKUPS + 1).exists());
}

#[test]
fn independent_writers_serialize_without_losing_or_corrupting_events() {
    let (_temp, paths) = fixture();
    let workers: Vec<_> = (0..8)
        .map(|worker| {
            let paths = paths.clone();
            std::thread::spawn(move || {
                let logger = Logger::new(&paths, Component::NativeHost).unwrap();
                for sequence in 0..100 {
                    logger
                        .write(
                            Level::Info,
                            Event::NativeRequestSucceeded,
                            Details {
                                request: Some(worker * 100 + sequence),
                                ..Default::default()
                            },
                            false,
                        )
                        .unwrap();
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    let entries = records(&log_path(&directory(&paths), Component::NativeHost, 0));
    let unique: std::collections::HashSet<_> =
        entries.iter().map(|r| r.details.request.unwrap()).collect();
    assert_eq!(entries.len(), 800);
    assert_eq!(unique.len(), 800);
}

#[test]
fn export_ignores_database_extra_fields_and_incomplete_lines_without_overwriting() {
    let (temp, paths) = fixture();
    let logger = Logger::new(&paths, Component::Desktop).unwrap();
    logger
        .write(Level::Info, Event::Ready, Details::default(), false)
        .unwrap();
    fs::write(paths.data.join("queue.sqlite3"), "DATABASE_SECRET").unwrap();
    fs::write(logger.root.join("unrelated.log"), "OTHER_SECRET").unwrap();
    let path = log_path(&logger.root, Component::Desktop, 0);
    let original = fs::read_to_string(&path).unwrap();
    let mut tampered: serde_json::Value = serde_json::from_str(original.trim()).unwrap();
    tampered["secret"] = "INJECTED_SECRET".into();
    let mut file = OpenOptions::new().append(true).open(&path).unwrap();
    writeln!(file, "{tampered}").unwrap();
    tampered.as_object_mut().unwrap().remove("secret");
    tampered["version"] = "VERSION_SECRET".into();
    writeln!(file, "{tampered}").unwrap();
    file.write_all(b"{\"time\": \"partial").unwrap();
    drop(file);
    let first = export(&paths, temp.path()).unwrap();
    let second = export(&paths, temp.path()).unwrap();
    assert_ne!(first, second);
    let content = fs::read_to_string(first).unwrap();
    assert!(!content.contains("SECRET"));
    let report: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(report["records"].as_array().unwrap().len(), 1);
    assert_eq!(report["skipped_entries"], 3);
    assert_eq!(
        fs::read_to_string(paths.data.join("queue.sqlite3")).unwrap(),
        "DATABASE_SECRET"
    );
}

#[test]
fn missing_unwritable_and_oversized_logs_report_errors() {
    let (temp, paths) = fixture();
    assert!(export(&paths, temp.path()).is_err());
    fs::create_dir_all(&paths.data).unwrap();
    fs::write(directory(&paths), "blocked").unwrap();
    assert!(Logger::new(&paths, Component::Desktop).is_err());
    fs::remove_file(directory(&paths)).unwrap();
    let logger = Logger::new(&paths, Component::Desktop).unwrap();
    let file = File::create(log_path(&logger.root, Component::Desktop, 0)).unwrap();
    file.set_len(MAX_FILE_BYTES + 1).unwrap();
    drop(file);
    assert!(
        export(&paths, temp.path())
            .unwrap_err()
            .to_string()
            .contains("size limit")
    );
    fs::write(
        log_path(&logger.root, Component::Desktop, 0),
        vec![b'a'; MAX_RECORD_BYTES + 1],
    )
    .unwrap();
    assert!(
        export(&paths, temp.path())
            .unwrap_err()
            .to_string()
            .contains("size limit")
    );
}

#[test]
fn panic_writer_does_not_wait_for_an_existing_lock() {
    let (_temp, paths) = fixture();
    let logger = Logger::new(&paths, Component::Desktop).unwrap();
    let _held = lock(&logger.root, Component::Desktop, false).unwrap();
    let started = std::time::Instant::now();
    assert!(
        logger
            .write(Level::Error, Event::Panic, Details::default(), true)
            .is_err()
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
}

#[test]
fn contended_export_is_bounded_and_recovers_when_the_lock_is_released() {
    let (_temp, paths) = fixture();
    let logger = Logger::new(&paths, Component::Desktop).unwrap();
    let held = lock(&logger.root, Component::Desktop, false).unwrap();
    let started = std::time::Instant::now();
    assert!(lock_mode(&logger.root, Component::Desktop, LockMode::Bounded).is_err());
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    drop(held);
    logger
        .write(Level::Info, Event::Ready, Details::default(), false)
        .unwrap();
    assert_eq!(
        records(&log_path(&logger.root, Component::Desktop, 0)).len(),
        1
    );
}

#[test]
fn appending_after_a_partial_write_keeps_the_next_event_readable() {
    let (temp, paths) = fixture();
    let logger = Logger::new(&paths, Component::Desktop).unwrap();
    let log = log_path(&logger.root, Component::Desktop, 0);
    fs::write(&log, b"{\"time\": \"interrupted").unwrap();
    logger
        .write(Level::Info, Event::Ready, Details::default(), false)
        .unwrap();
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(export(&paths, temp.path()).unwrap()).unwrap()).unwrap();
    assert_eq!(report["records"][0]["event"], "ready");
    assert_eq!(report["skipped_entries"], 1);
}

#[test]
fn viewer_has_an_empty_state_and_does_not_create_missing_logs() {
    let (_temp, paths) = fixture();
    let snapshot = read_recent(&paths).unwrap();
    assert!(snapshot.records.is_empty());
    assert_eq!(snapshot.skipped, 0);
    assert_eq!(snapshot.older, 0);
    assert!(!directory(&paths).exists());
}

#[test]
fn viewer_merges_rotation_and_components_in_actual_time_order() {
    let (_temp, paths) = fixture();
    let logger = Logger::new(&paths, Component::Desktop).unwrap();
    let cases = [
        (
            Component::Desktop,
            1,
            "2026-10-14T09:00:00+08:00",
            Event::Started,
        ),
        (Component::Desktop, 0, "2026-10-14T01:10:00Z", Event::Ready),
        (
            Component::NativeHost,
            0,
            "2026-10-14T01:20:00Z",
            Event::NativeRequestReceived,
        ),
    ];
    for (component, index, time, event) in cases {
        let mut record = logger.entry(Level::Info, event, Details::default());
        record.component = component;
        record.time = time.into();
        fs::write(
            log_path(&logger.root, component, index),
            format!("{}\n", serde_json::to_string(&record).unwrap()),
        )
        .unwrap();
    }
    let snapshot = read_recent(&paths).unwrap();
    assert_eq!(
        snapshot.records.iter().map(|r| r.event).collect::<Vec<_>>(),
        vec![Event::NativeRequestReceived, Event::Ready, Event::Started]
    );
    assert_eq!(snapshot.records[0].component, Component::NativeHost);
}

#[test]
fn viewer_caps_newest_entries_without_limiting_export() {
    let (temp, paths) = fixture();
    let logger = Logger::new(&paths, Component::Desktop).unwrap();
    let mut contents = Vec::new();
    for index in 0..RECENT_LIMIT + 7 {
        let mut record = logger.entry(
            Level::Info,
            Event::DownloadStarted,
            Details {
                request: Some(index as u64),
                ..Default::default()
            },
        );
        record.time = "2026-10-14T01:00:00Z".into();
        serde_json::to_writer(&mut contents, &record).unwrap();
        contents.push(b'\n');
    }
    fs::write(log_path(&logger.root, Component::Desktop, 0), contents).unwrap();
    let snapshot = read_recent(&paths).unwrap();
    assert_eq!(snapshot.records.len(), RECENT_LIMIT);
    assert_eq!(snapshot.older, 7);
    assert_eq!(
        snapshot.records[0].details.request,
        Some((RECENT_LIMIT + 6) as u64)
    );
    assert_eq!(snapshot.records.last().unwrap().details.request, Some(7));
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(export(&paths, temp.path()).unwrap()).unwrap()).unwrap();
    assert_eq!(
        report["records"].as_array().unwrap().len(),
        RECENT_LIMIT + 7
    );
}

#[test]
fn viewer_rejects_untrusted_fields_and_bounds_io() {
    let (_temp, paths) = fixture();
    let logger = Logger::new(&paths, Component::Desktop).unwrap();
    let record = logger.entry(Level::Error, Event::DownloadFailed, Details::default());
    let valid = serde_json::to_string(&record).unwrap();
    let mut injected = serde_json::to_value(&record).unwrap();
    injected["details"]["message"] = "SECRET URL".into();
    let path = log_path(&logger.root, Component::Desktop, 0);
    fs::write(&path, format!("{valid}\n{injected}\n{{\"partial\":")).unwrap();
    let snapshot = read_recent(&paths).unwrap();
    assert_eq!(snapshot.records, vec![record]);
    assert_eq!(snapshot.skipped, 2);
    assert!(!format!("{snapshot:?}").contains("SECRET"));
    let lock = lock_mode(&logger.root, Component::Desktop, LockMode::Try).unwrap();
    let start = std::time::Instant::now();
    assert!(read_recent(&paths).is_err());
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
    drop(lock);
    assert_eq!(read_recent(&paths).unwrap().records.len(), 1);
    fs::write(&path, vec![b'x'; MAX_RECORD_BYTES + 1]).unwrap();
    assert!(
        read_recent(&paths)
            .unwrap_err()
            .to_string()
            .contains("size limit")
    );
    File::create(path)
        .unwrap()
        .set_len(MAX_FILE_BYTES + 1)
        .unwrap();
    assert!(
        read_recent(&paths)
            .unwrap_err()
            .to_string()
            .contains("size limit")
    );
}
