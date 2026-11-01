//! Verify cross-process exclusion, compatibility with the former Windows lock
//! range, and recovery after a lock holder is terminated. All files are temporary.
#![cfg(windows)]

use std::{
    fs::{File, OpenOptions, TryLockError},
    io::{BufRead, BufReader, Write},
    os::windows::{io::AsRawHandle, process::CommandExt},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread::JoinHandle,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{ERROR_LOCK_VIOLATION, HANDLE},
    Storage::FileSystem::{
        LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx, UnlockFile,
    },
    System::{IO::OVERLAPPED, Threading::CREATE_NO_WINDOW},
};

struct Peer {
    child: Child,
    replies: Receiver<String>,
    reader: Option<JoinHandle<()>>,
}

impl Peer {
    fn start(path: &Path, legacy: bool) -> Self {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "lock_process_fixture",
                "--nocapture",
            ])
            .env("DOWNLOAD_ENGINE_LOCK_TEST_PATH", path)
            .env(
                "DOWNLOAD_ENGINE_LOCK_TEST_LEGACY",
                if legacy { "1" } else { "0" },
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, replies) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if let Some((_, reply)) = line.split_once("LOCK:")
                    && sender.send(reply.to_owned()).is_err()
                {
                    break;
                }
            }
        });
        let peer = Self {
            child,
            replies,
            reader: Some(reader),
        };
        peer.expect("ready");
        peer
    }

    fn send(&mut self, command: &str) {
        let input = self.child.stdin.as_mut().unwrap();
        writeln!(input, "{command}").unwrap();
        input.flush().unwrap();
    }

    fn expect(&self, expected: &str) {
        assert_eq!(
            self.replies.recv_timeout(Duration::from_secs(5)).unwrap(),
            expected
        );
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            reader.join().unwrap();
        }
    }
}

#[test]
fn native_and_legacy_locks_exclude_each_other_and_release_after_process_exit() {
    for legacy_owner in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("profile.lock");
        std::fs::write(&path, b"keep existing lock-file contents").unwrap();
        let mut owner = Peer::start(&path, legacy_owner);
        let mut contender = Peer::start(&path, !legacy_owner);
        owner.send("try");
        owner.expect("acquired");
        contender.send("try");
        contender.expect("busy");
        owner.send("unlock");
        owner.expect("unlocked");
        contender.send("try");
        contender.expect("acquired");
        drop(contender); // Simulate a crash: no explicit unlock is performed.
        owner.send("try");
        owner.expect("acquired");
        owner.send("unlock");
        owner.expect("unlocked");
        drop(owner);
        assert_eq!(
            std::fs::read(path).unwrap(),
            b"keep existing lock-file contents"
        );
    }
}

#[test]
fn blocking_native_lock_waits_for_legacy_holder_and_wakes_on_unlock() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("migration.lock");
    let mut owner = Peer::start(&path, true);
    let mut waiter = Peer::start(&path, false);
    owner.send("try");
    owner.expect("acquired");
    waiter.send("wait");
    waiter.expect("waiting");
    assert!(matches!(
        waiter.replies.recv_timeout(Duration::from_millis(150)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    owner.send("unlock");
    owner.expect("unlocked");
    waiter.expect("acquired");
    owner.send("try");
    owner.expect("busy");
    waiter.send("unlock");
    waiter.expect("unlocked");
    owner.send("try");
    owner.expect("acquired");
}

fn legacy_try_lock(file: &File) -> Result<(), TryLockError> {
    // Match fs2's Windows implementation: exclusive, nonblocking, offset zero,
    // with both length words set to MAX. The live File owns the handle.
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    let result = unsafe {
        LockFileEx(
            file.as_raw_handle() as HANDLE,
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        )
    };
    if result != 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32) {
        Err(TryLockError::WouldBlock)
    } else {
        Err(TryLockError::Error(error))
    }
}

// Invoked only by the parent tests, in a separate process with temporary paths.
#[test]
#[ignore]
fn lock_process_fixture() {
    let path = std::env::var_os("DOWNLOAD_ENGINE_LOCK_TEST_PATH").unwrap();
    let legacy = std::env::var("DOWNLOAD_ENGINE_LOCK_TEST_LEGACY").unwrap() == "1";
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    let reply = |value: &str| {
        println!("LOCK:{value}");
        std::io::stdout().flush().unwrap();
    };
    reply("ready");
    for command in std::io::stdin().lock().lines() {
        match command.unwrap().as_str() {
            "try" => match if legacy {
                legacy_try_lock(&file)
            } else {
                file.try_lock()
            } {
                Ok(()) => reply("acquired"),
                Err(TryLockError::WouldBlock) => reply("busy"),
                Err(error) => panic!("Lock I/O failure: {error}"),
            },
            "wait" => {
                assert!(!legacy);
                reply("waiting");
                file.lock().unwrap();
                reply("acquired");
            }
            "unlock" => {
                if legacy {
                    // The file remains live and this process owns the same range.
                    assert_ne!(
                        unsafe {
                            UnlockFile(file.as_raw_handle() as HANDLE, 0, 0, u32::MAX, u32::MAX)
                        },
                        0
                    );
                } else {
                    file.unlock().unwrap();
                }
                reply("unlocked");
            }
            command => panic!("Unexpected fixture command: {command}"),
        }
    }
}
