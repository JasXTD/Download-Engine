use download_engine::{crash_reports, diagnostics, paths::AppPaths};
use std::{fs, process::Command};

#[test]
fn crash_reports_survive_panics_and_forced_exit_without_secrets_or_duplicates() {
    for mode in [
        "clean",
        "panic",
        "worker",
        "parallel",
        "host",
        "blocked-log",
        "forced-exit",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths {
            data: temp.path().join("data"),
            downloads: temp.path().join("downloads"),
        };
        let result = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "crash_report_process_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("DE_CRASH_TEST_ROOT", temp.path())
            .env("DE_CRASH_TEST_MODE", mode)
            .env("DE_TEST_SECRET", "SECRET_environment")
            .output()
            .unwrap();
        assert_eq!(
            result.status.success(),
            matches!(mode, "clean" | "worker" | "parallel"),
            "{mode}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        // Startup after the process is gone is exactly when unclean exits are detected.
        drop(crash_reports::DesktopSession::begin(&paths).unwrap());
        let snapshot = crash_reports::load(&paths).unwrap();
        assert_eq!(
            snapshot.reports.len(),
            usize::from(mode != "clean"),
            "{mode}"
        );
        for saved in snapshot.reports {
            let report = saved.report;
            assert_eq!(
                report.component,
                if mode == "host" {
                    diagnostics::Component::NativeHost
                } else {
                    diagnostics::Component::Desktop
                }
            );
            assert_eq!(
                report.kind,
                if mode == "forced-exit" {
                    crash_reports::Kind::UnexpectedExit
                } else {
                    crash_reports::Kind::RustPanic
                }
            );
            assert_eq!(report.location.is_some(), mode != "forced-exit");
            let exported = crash_reports::export(&report, temp.path()).unwrap();
            let text = fs::read_to_string(exported).unwrap();
            for secret in [
                "SECRET",
                "example.com",
                "Alice",
                &temp.path().display().to_string(),
            ] {
                assert!(!text.contains(secret), "{mode} leaked {secret}");
            }
            assert_eq!(text, crash_reports::preview(&report).unwrap());
            crash_reports::acknowledge(&paths, &[report.id]).unwrap();
        }
        drop(crash_reports::DesktopSession::begin(&paths).unwrap());
        assert!(
            crash_reports::load(&paths)
                .unwrap()
                .reports
                .iter()
                .all(|r| !r.unreviewed)
        );
    }
}

// A separate test process owns the global hook and a disposable profile only.
#[test]
#[ignore]
fn crash_report_process_fixture() {
    let root = std::path::PathBuf::from(std::env::var_os("DE_CRASH_TEST_ROOT").unwrap());
    let paths = AppPaths {
        data: root.join("data"),
        downloads: root.join("downloads"),
    };
    let mode = std::env::var("DE_CRASH_TEST_MODE").unwrap();
    let component = if mode == "host" {
        diagnostics::Component::NativeHost
    } else {
        diagnostics::Component::Desktop
    };
    diagnostics::init(&paths, component).unwrap();
    diagnostics::flush().unwrap();
    let _session = (mode != "host").then(|| crash_reports::DesktopSession::begin(&paths).unwrap());
    if mode == "forced-exit" {
        std::process::exit(37);
    }
    if mode == "clean" {
        return;
    }
    if mode == "parallel" {
        let gate = std::sync::Arc::new(std::sync::Barrier::new(3));
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let gate = gate.clone();
                std::thread::spawn(move || {
                    gate.wait();
                    panic!("SECRET_parallel");
                })
            })
            .collect();
        gate.wait();
        for worker in workers {
            assert!(worker.join().is_err());
        }
        return;
    }
    if mode == "worker" {
        let _ =
            std::thread::spawn(|| panic!("SECRET_worker https://alice:SECRET@example.com/private"))
                .join();
        return;
    }
    let _log_lock = if mode == "blocked-log" {
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(paths.data.join("logs/desktop.lock"))
            .unwrap();
        file.lock().unwrap();
        Some(file)
    } else {
        None
    };
    panic!("SECRET_payload C:\\Users\\Alice\\private.zip https://example.com/?token=SECRET");
}
