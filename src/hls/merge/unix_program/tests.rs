use super::super::Helper;
use super::*;
use std::{fs, path::Path};

fn status(command: &mut Command) -> std::io::Result<std::process::ExitStatus> {
    Helper::start(command, &CancellationToken::new())?.wait()
}

fn fixture() -> (tempfile::TempDir, PathBuf, String) {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("ffmpeg");
    // Harmless native OS fixtures: no shell, network or application data.
    fs::copy("/usr/bin/true", &source).unwrap();
    let expected = format!("{:x}", Sha256::digest(fs::read(&source).unwrap()));
    (root, source, expected)
}

fn checked(source: &Path, expected: &str) -> Arc<Program> {
    Program::verified(
        File::open(source).unwrap(),
        expected,
        &CancellationToken::new(),
    )
    .unwrap()
}

#[test]
fn permissive_umask_keeps_the_snapshot_private_and_executable() {
    const CHILD: &str = "DOWNLOAD_ENGINE_PRIVATE_MERGER_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "hls::merge::unix_program::tests::permissive_umask_keeps_the_snapshot_private_and_executable",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    // umask is process-global: change it only in this isolated test child.
    unsafe { libc::umask(0) };
    let (_root, source, expected) = fixture();
    let program = checked(&source, &expected);
    let folder = program.folder.clone();
    assert_eq!(fs::metadata(&folder).unwrap().mode() & 0o777, 0o700);
    assert_eq!(
        fs::metadata(folder.join("ffmpeg")).unwrap().mode() & 0o777,
        0o500
    );
    assert!(status(&mut program.command()).unwrap().success());
    drop(program);
    assert!(!folder.exists());
}

#[test]
fn replacing_the_resource_cannot_replace_the_verified_program() {
    let (_root, source, expected) = fixture();
    let program = checked(&source, &expected);
    fs::rename(&source, source.with_extension("old")).unwrap();
    fs::copy("/usr/bin/false", &source).unwrap();
    // Prove the replacement is executable and distinguishable from the pin.
    assert!(!Command::new(&source).status().unwrap().success());
    assert!(status(&mut program.command()).unwrap().success());
}

#[test]
fn rewriting_the_original_inode_does_not_change_the_verified_copy() {
    let (_root, source, expected) = fixture();
    let program = checked(&source, &expected);
    let before = fs::metadata(&source).unwrap().ino();
    fs::write(&source, fs::read("/usr/bin/false").unwrap()).unwrap();
    assert_eq!(fs::metadata(&source).unwrap().ino(), before);
    assert!(!Command::new(&source).status().unwrap().success());
    assert!(status(&mut program.command()).unwrap().success());
}

#[test]
fn replacing_resource_ancestors_or_the_callers_cwd_cannot_redirect_exec() {
    let (root, source, expected) = fixture();
    let resource = root.path().join("resources");
    fs::create_dir(&resource).unwrap();
    fs::rename(&source, resource.join("ffmpeg")).unwrap();
    let program = checked(&resource.join("ffmpeg"), &expected);
    fs::rename(&resource, root.path().join("old-resources")).unwrap();
    fs::create_dir(&resource).unwrap();
    fs::copy("/usr/bin/false", resource.join("ffmpeg")).unwrap();
    assert!(
        status(program.command().current_dir(&resource))
            .unwrap()
            .success()
    );
}

#[test]
fn replacing_the_snapshot_name_cannot_redirect_exec_or_recursive_cleanup() {
    let (root, source, expected) = fixture();
    let program = checked(&source, &expected);
    let original_name = program.folder.clone();
    let moved = root.path().join("moved-snapshot");
    fs::rename(&original_name, &moved).unwrap();
    fs::create_dir(&original_name).unwrap();
    fs::copy("/usr/bin/false", original_name.join("ffmpeg")).unwrap();
    fs::write(original_name.join("keep"), b"preserve replacement files").unwrap();
    assert!(status(&mut program.command()).unwrap().success());
    drop(program);
    assert_eq!(
        fs::read(original_name.join("keep")).unwrap(),
        b"preserve replacement files"
    );
    assert!(!moved.join("ffmpeg").exists());
    // Only this test's two replacement files and its empty directory.
    fs::remove_file(original_name.join("ffmpeg")).unwrap();
    fs::remove_file(original_name.join("keep")).unwrap();
    fs::remove_dir(original_name).unwrap();
}

#[test]
fn commands_keep_the_snapshot_alive_and_release_it_after_completion_or_spawn_failure() {
    for fail_spawn in [false, true] {
        let (root, source, expected) = fixture();
        let program = checked(&source, &expected);
        let folder = program.folder.clone();
        let mut command = program.command();
        drop(program);
        assert!(folder.is_dir());
        if fail_spawn {
            command.current_dir(root.path().join("missing"));
            assert!(status(&mut command).is_err());
        } else {
            assert!(status(&mut command).unwrap().success());
        }
        drop(command);
        assert!(!folder.exists());
    }
}

#[test]
fn bad_checksums_cancellation_and_oversized_helpers_fail_before_execution() {
    let (_root, source, expected) = fixture();
    assert!(
        Program::verified(
            File::open(&source).unwrap(),
            &"0".repeat(64),
            &CancellationToken::new()
        )
        .is_err()
    );
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(Program::verified(File::open(&source).unwrap(), &expected, &cancel).is_err());
    File::options()
        .write(true)
        .open(&source)
        .unwrap()
        .set_len(MAX_PROGRAM_BYTES + 1)
        .unwrap();
    assert!(
        Program::verified(
            File::open(&source).unwrap(),
            &expected,
            &CancellationToken::new()
        )
        .is_err()
    );
}

#[test]
fn guarded_child_launch_keeps_the_verified_program_and_process_limits() {
    let (_root, source, expected) = fixture();
    let program = checked(&source, &expected);
    let token = CancellationToken::new();
    let mut command = program.command();
    let mut running = Helper::spawn(&mut command, &token, Some(64 << 20)).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let status = loop {
        if let Some(status) = running.try_wait().unwrap() {
            break status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "fixture failed to exit"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    assert!(status.success());
    token.cancel();
    assert!(Helper::spawn(&mut command, &token, Some(64 << 20)).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn a_busy_verified_helper_waits_for_the_writer_or_cancellation() {
    use std::{io::ErrorKind, sync::mpsc, time::Duration};

    let (_root, source, expected) = fixture();
    let program = checked(&source, &expected);
    let path = program.folder.join("ffmpeg");
    for cancel_launch in [false, true] {
        // Keep one writable descriptor open, as a sibling process can between
        // fork and exec. Restore the executable's normal mode before launch.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let writer = File::options().write(true).open(&path).unwrap();
        writer
            .set_permissions(fs::Permissions::from_mode(0o500))
            .unwrap();
        assert_eq!(
            program.command().spawn().unwrap_err().kind(),
            ErrorKind::ExecutableFileBusy
        );

        let cancel = CancellationToken::new();
        std::thread::scope(|scope| {
            let (send, receive) = mpsc::channel();
            let (ready, started) = mpsc::channel();
            let program = &program;
            let token = cancel.clone();
            scope.spawn(move || {
                ready.send(()).unwrap();
                let result = Helper::start(&mut program.command(), &token)
                    .and_then(|mut child| child.wait());
                send.send(result).unwrap();
            });
            started.recv().unwrap();
            assert!(matches!(
                receive.recv_timeout(Duration::from_millis(50)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ));
            if cancel_launch {
                cancel.cancel();
                assert_eq!(
                    receive
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap()
                        .unwrap_err()
                        .kind(),
                    ErrorKind::Interrupted
                );
                drop(writer);
            } else {
                drop(writer);
                assert!(
                    receive
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap()
                        .unwrap()
                        .success()
                );
            }
        });
    }
}
