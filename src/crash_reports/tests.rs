use super::*;

fn fixture() -> (tempfile::TempDir, AppPaths) {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    };
    fs::create_dir_all(directory(&paths)).unwrap();
    (temp, paths)
}

#[test]
fn reports_export_only_reviewed_metadata_and_never_overwrite_a_file() {
    let (temp, paths) = fixture();
    let report = Report::new(Component::Desktop, Uuid::new_v4());
    let path = report_path(&directory(&paths), report.id, false);
    write_new(&path, &report).unwrap();
    assert!(write_new(&path, &Report::new(Component::NativeHost, Uuid::new_v4())).is_err());
    let loaded = load(&paths).unwrap();
    assert_eq!(loaded.reports[0].report, report);
    let text = preview(&report).unwrap();
    let first = export(&report, temp.path()).unwrap();
    let second = export(&report, temp.path()).unwrap();
    assert_ne!(first, second);
    assert_eq!(fs::read_to_string(first).unwrap(), text);
    assert!(!text.contains(&temp.path().display().to_string()));
    assert!(!text.contains("downloads"));
    acknowledge(&paths, &[report.id]).unwrap();
    acknowledge(&paths, &[report.id]).unwrap();
    let loaded = load(&paths).unwrap();
    assert_eq!(loaded.reports.len(), 1);
    assert!(!loaded.reports[0].unreviewed);
    assert_eq!(loaded.reports[0].report, report);
}

#[test]
fn malformed_oversized_redirected_and_secret_bearing_reports_are_not_exported() {
    let (temp, paths) = fixture();
    let report = Report::new(Component::Desktop, Uuid::new_v4());
    let path = report_path(&directory(&paths), report.id, false);
    let mut value = serde_json::to_value(&report).unwrap();
    value["panic_message"] = "https://alice:SECRET@example.com/private?token=SECRET".into();
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(load(&paths).unwrap().skipped, 1);
    assert!(load(&paths).unwrap().reports.is_empty());
    fs::write(&path, vec![b'x'; MAX_BYTES as usize + 1]).unwrap();
    assert_eq!(load(&paths).unwrap().skipped, 1);
    fs::remove_file(&path).unwrap();
    let outside = temp.path().join("private.json");
    fs::write(&outside, serde_json::to_vec(&report).unwrap()).unwrap();
    fs::hard_link(&outside, &path).unwrap();
    assert_eq!(load(&paths).unwrap().skipped, 1);
    assert!(acknowledge(&paths, &[report.id]).is_err());
    assert!(outside.exists());
    for file in [
        "C:/Users/Alice/SECRET.rs",
        "https://example.com/secret.rs",
        "src/../SECRET.rs",
        "src/private@address.rs",
    ] {
        let mut unsafe_report = report.clone();
        unsafe_report.kind = Kind::RustPanic;
        unsafe_report.location = Some(SourceLocation {
            file: file.into(),
            line: 1,
            column: 1,
        });
        assert!(preview(&unsafe_report).is_err());
        assert!(export(&unsafe_report, temp.path()).is_err());
    }
}

#[test]
fn retention_is_bounded_and_leaves_unrelated_files_alone() {
    let (_temp, paths) = fixture();
    let root = directory(&paths);
    let unrelated = root.join("my-notes.json");
    fs::write(&unrelated, "keep").unwrap();
    let mut oldest = None;
    for day in 1..=30 {
        let mut report = Report::new(Component::Desktop, Uuid::new_v4());
        report.recorded_utc = format!("2026-10-{day:02}T00:00:00.000Z");
        let path = report_path(&root, report.id, day % 2 == 0);
        if day == 1 {
            oldest = Some(path.clone());
        }
        write_new(&path, &report).unwrap();
    }
    retain_recent(&root).unwrap();
    let snapshot = load(&paths).unwrap();
    assert_eq!(snapshot.reports.len(), RETAINED_REPORTS);
    assert!(!oldest.unwrap().exists());
    assert_eq!(
        snapshot.reports[0].report.recorded_utc,
        "2026-10-30T00:00:00.000Z"
    );
    assert_eq!(fs::read_to_string(unrelated).unwrap(), "keep");
}

#[test]
fn clean_sessions_do_not_report_and_unclean_sessions_report_once() {
    let (_temp, paths) = fixture();
    drop(DesktopSession::begin(&paths).unwrap());
    assert!(load(&paths).unwrap().reports.is_empty());
    let session = DesktopSession::begin(&paths).unwrap();
    let id = session.id;
    std::mem::forget(session); // the process test also exercises a real forced exit
    drop(DesktopSession::begin(&paths).unwrap());
    let first = load(&paths).unwrap();
    assert_eq!(first.reports.len(), 1);
    assert_eq!(first.reports[0].report.id, id);
    assert_eq!(first.reports[0].report.kind, Kind::UnexpectedExit);
    drop(DesktopSession::begin(&paths).unwrap());
    assert_eq!(load(&paths).unwrap().reports.len(), 1);
}

#[test]
fn a_panic_and_its_unclean_session_produce_one_report() {
    let (_temp, paths) = fixture();
    let session = DesktopSession::begin(&paths).unwrap();
    let mut report = read_report(&directory(&paths).join("desktop-session.json")).unwrap();
    report.kind = Kind::RustPanic;
    write_new(&report_path(&directory(&paths), report.id, false), &report).unwrap();
    std::mem::forget(session);
    drop(DesktopSession::begin(&paths).unwrap());
    let snapshot = load(&paths).unwrap();
    assert_eq!(snapshot.reports.len(), 1);
    assert_eq!(snapshot.reports[0].report.kind, Kind::RustPanic);
}

#[test]
fn write_failures_are_explicit_and_existing_data_survives() {
    let (temp, paths) = fixture();
    let report = Report::new(Component::Desktop, Uuid::new_v4());
    let occupied = temp.path().join("occupied");
    fs::write(&occupied, "keep").unwrap();
    assert!(export(&report, &occupied).is_err());
    assert_eq!(fs::read_to_string(occupied).unwrap(), "keep");
    let marker = directory(&paths).join("desktop-session.json");
    fs::write(&marker, "private corrupt marker").unwrap();
    assert!(DesktopSession::begin(&paths).is_err());
    assert_eq!(
        fs::read_to_string(marker).unwrap(),
        "private corrupt marker"
    );
}

#[test]
fn application_source_location_is_relative() {
    let location = source_location(std::panic::Location::caller()).unwrap();
    assert!(location.file.ends_with("crash_reports/tests.rs"));
    assert!(!location.file.contains(':'));
    assert!(!location.file.contains("Users"));
}

#[cfg(unix)]
#[test]
fn fifo_and_symlink_reports_never_block_or_follow_targets() {
    use std::os::unix::ffi::OsStrExt;
    let (temp, paths) = fixture();
    let id = Uuid::new_v4();
    let path = report_path(&directory(&paths), id, false);
    let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert_eq!(load(&paths).unwrap().skipped, 1);
    fs::remove_file(&path).unwrap();
    let target = temp.path().join("private");
    fs::write(&target, "secret").unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert_eq!(load(&paths).unwrap().skipped, 1);
    assert_eq!(fs::read_to_string(target).unwrap(), "secret");
}
