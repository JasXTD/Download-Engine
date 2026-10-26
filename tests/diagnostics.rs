//! Exercise real native processes and their stdout framing, not mocked log sinks.
use download_engine::{diagnostics, native, paths::AppPaths, store::Store};
use std::{
    io::Write,
    process::{Command, Stdio},
};

fn host(home: &std::path::Path, requests: &[serde_json::Value]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_de-host"))
        .env("IDM_EX_HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    for request in requests {
        let bytes = serde_json::to_vec(request).unwrap();
        input
            .write_all(&(bytes.len() as u32).to_le_bytes())
            .unwrap();
        input.write_all(&bytes).unwrap();
    }
    drop(input);
    child.wait_with_output().unwrap()
}

#[test]
fn real_connectors_share_logs_without_contaminating_native_stdout_or_saving_secrets() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    };
    drop(Store::open(paths.clone()).unwrap());
    let workers: Vec<_> = (0..6).map(|_| {
        let home = temp.path().to_owned();
        std::thread::spawn(move || {
            let result = host(&home, &[
                serde_json::json!({"action":"ping"}),
                serde_json::json!({"action":"add", "request_id":"SECRET_REQUEST", "url":"invalid-SECRET_URL"}),
                serde_json::json!({"action":"secret-UNKNOWN_ACTION", "cookie":"SECRET_COOKIE"}),
            ]);
            assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
            let mut stdout = std::io::Cursor::new(result.stdout);
            for expected in [true, false, false] {
                let data = native::read_frame(&mut stdout).unwrap().unwrap();
                let reply: native::Reply = serde_json::from_slice(&data).unwrap();
                assert_eq!(reply.ok, expected);
            }
            assert!(native::read_frame(&mut stdout).unwrap().is_none());
        })
    }).collect();
    for worker in workers {
        worker.join().unwrap();
    }
    let export = diagnostics::export(&paths, temp.path()).unwrap();
    let text = std::fs::read_to_string(export).unwrap();
    assert!(!text.contains("SECRET"));
    let report: serde_json::Value = serde_json::from_str(&text).unwrap();
    let records = report["records"].as_array().unwrap();
    let count = |event| {
        records
            .iter()
            .filter(|record| record["event"] == event)
            .count()
    };
    assert_eq!(count("started"), 6);
    assert_eq!(count("native_request_received"), 18);
    assert_eq!(count("native_request_succeeded"), 6);
    assert_eq!(count("native_message_invalid"), 6);
    assert_eq!(count("stopped"), 6);
}

#[test]
fn unavailable_log_directory_does_not_break_browser_connection() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("data")).unwrap();
    std::fs::write(temp.path().join("data/logs"), "blocked").unwrap();
    let result = host(temp.path(), &[serde_json::json!({"action":"ping"})]);
    assert!(result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("diagnostics are unavailable"));
    let data = native::read_frame(&mut std::io::Cursor::new(result.stdout))
        .unwrap()
        .unwrap();
    assert!(serde_json::from_slice::<native::Reply>(&data).unwrap().ok);
}

#[test]
fn native_startup_failures_are_persisted_without_polluting_stdout() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("data/queue.sqlite3")).unwrap();
    let result = host(temp.path(), &[]);
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    let text = std::fs::read_to_string(temp.path().join("data/logs/native-host.log")).unwrap();
    assert!(text.contains("native_host_failed"));
    assert!(text.contains("database"));
    assert!(!text.contains(&temp.path().display().to_string()));
}

#[test]
fn desktop_panics_and_real_transfer_outcomes_are_logged_in_isolated_processes() {
    for mode in ["panic", "transfer", "blocked"] {
        let temp = tempfile::tempdir().unwrap();
        let result = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "diagnostics_process_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("IDM_EX_DIAGNOSTIC_FIXTURE", temp.path())
            .env("IDM_EX_DIAGNOSTIC_MODE", mode)
            .output()
            .unwrap();
        assert_eq!(
            result.status.success(),
            mode != "panic",
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let content = std::fs::read_to_string(temp.path().join("data/logs/desktop.log")).unwrap();
        assert!(!content.contains("SECRET"));
        let entries: Vec<serde_json::Value> = content
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        if mode == "panic" {
            assert!(entries.iter().any(|r| r["event"] == "panic"));
        } else if mode == "blocked" {
            let accepted = entries.iter().filter(|r| r["event"] == "ready").count() as u64;
            let dropped: u64 = entries
                .iter()
                .filter(|r| r["event"] == "entries_dropped")
                .map(|r| r["details"]["dropped"].as_u64().unwrap())
                .sum();
            assert!(dropped > 0);
            assert_eq!(accepted + dropped, 2000);
        } else {
            assert!(
                entries
                    .iter()
                    .any(|r| r["event"] == "download_completed" && r["details"]["bytes"] == 4)
            );
            assert!(
                entries
                    .iter()
                    .any(|r| r["event"] == "download_failed" && r["level"] == "error")
            );
        }
    }
}

// Invoked only by the test above: isolates the process-global logger and panic hook.
#[test]
#[ignore]
fn diagnostics_process_fixture() {
    use download_engine::{
        bandwidth::Limiter,
        engine,
        model::{Job, Status},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let root = std::path::PathBuf::from(std::env::var_os("IDM_EX_DIAGNOSTIC_FIXTURE").unwrap());
    let paths = AppPaths {
        data: root.join("data"),
        downloads: root.join("downloads"),
    };
    diagnostics::init(&paths, diagnostics::Component::Desktop).unwrap();
    diagnostics::flush().unwrap();
    if std::env::var("IDM_EX_DIAGNOSTIC_MODE").unwrap() == "panic" {
        panic!("SECRET panic payload");
    }
    if std::env::var("IDM_EX_DIAGNOSTIC_MODE").unwrap() == "blocked" {
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(paths.data.join("logs/desktop.lock"))
            .unwrap();
        fs2::FileExt::lock_exclusive(&lock).unwrap();
        let (done, receiver) = std::sync::mpsc::channel();
        let producer = std::thread::spawn(move || {
            for request in 0..2000 {
                diagnostics::record(
                    diagnostics::Level::Info,
                    diagnostics::Event::Ready,
                    diagnostics::Details {
                        request: Some(request),
                        ..Default::default()
                    },
                );
            }
            done.send(()).unwrap();
        });
        // The producer must finish while disk writing is still blocked.
        let finished = receiver.recv_timeout(std::time::Duration::from_secs(2));
        drop(lock);
        producer.join().unwrap();
        finished.unwrap();
        diagnostics::flush().unwrap();
        return;
    }
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0u8; 8192];
                let count = socket.read(&mut request).await.unwrap();
                let success = String::from_utf8_lossy(&request[..count]).contains("/success");
                let response = if success {
                    "HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndata"
                } else {
                    "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                };
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let store = Store::open(paths).unwrap();
        for (suffix, expected) in [("success", Status::Complete), ("failure", Status::Failed)] {
            let mut job = Job::new(
                &format!("http://{address}/{suffix}?token=SECRET"),
                Some("SECRET-filename.bin"),
                store.paths.downloads.clone(),
                uuid::Uuid::new_v4().to_string(),
            )
            .unwrap();
            job.status = Status::Downloading;
            let job = store.enqueue(&job).unwrap();
            let id = job.id.clone();
            engine::run(
                store.clone(),
                job,
                tokio_util::sync::CancellationToken::new(),
                Limiter::new(&store.bandwidth_limits().unwrap()).unwrap(),
            )
            .await
            .unwrap();
            assert_eq!(store.get(&id).unwrap().unwrap().status, expected);
        }
        server.await.unwrap();
    });
    diagnostics::flush().unwrap();
}
