use super::*;
use std::{fs, path::Path};

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
fn replacing_the_resource_cannot_replace_the_verified_program() {
    let (_root, source, expected) = fixture();
    let program = checked(&source, &expected);
    fs::rename(&source, source.with_extension("old")).unwrap();
    fs::copy("/usr/bin/false", &source).unwrap();
    // Prove the replacement is executable and distinguishable from the pin.
    assert!(!Command::new(&source).status().unwrap().success());
    assert!(program.command().status().unwrap().success());
}

#[test]
fn rewriting_the_original_inode_does_not_change_the_verified_copy() {
    let (_root, source, expected) = fixture();
    let program = checked(&source, &expected);
    let before = fs::metadata(&source).unwrap().ino();
    fs::write(&source, fs::read("/usr/bin/false").unwrap()).unwrap();
    assert_eq!(fs::metadata(&source).unwrap().ino(), before);
    assert!(!Command::new(&source).status().unwrap().success());
    assert!(program.command().status().unwrap().success());
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
        program
            .command()
            .current_dir(&resource)
            .status()
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
    assert!(program.command().status().unwrap().success());
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
            assert!(command.status().is_err());
        } else {
            assert!(command.status().unwrap().success());
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
    let mut running =
        crate::child_process::Running::spawn(&mut command, &token, Some(64 << 20)).unwrap();
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
    assert!(crate::child_process::Running::spawn(&mut command, &token, Some(64 << 20)).is_err());
}
