//! Bounded output and owned child-process lifetime for scanners and the media merger.
use anyhow::{Context, Result, ensure};
use std::{
    io::Read,
    process::{Child, Command},
    sync::mpsc,
};
use tokio_util::sync::CancellationToken;

pub(crate) fn reader(
    mut pipe: impl Read + Send + 'static,
) -> mpsc::Receiver<Result<(String, bool)>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| -> Result<(String, bool)> {
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            let mut truncated = false;
            loop {
                let count = pipe.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                let kept = count.min(16384 - bytes.len());
                truncated |= count > kept;
                bytes.extend_from_slice(&buffer[..kept]);
            }
            Ok((String::from_utf8_lossy(&bytes).into_owned(), truncated))
        })();
        let _ = tx.send(result);
    });
    rx
}

pub(crate) struct Running {
    pub(crate) child: Child,
    #[cfg(windows)]
    tree: Option<std::os::windows::io::OwnedHandle>,
}
impl Drop for Running {
    fn drop(&mut self) {
        #[cfg(windows)]
        drop(self.tree.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
#[cfg(windows)]
fn process_tree(
    child: &Child,
    memory_limit: Option<usize>,
) -> Result<std::os::windows::io::OwnedHandle> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::JobObjects::*;
    // An unnamed, non-inheritable Job owns only our helper and its children.
    // Closing it also covers application shutdown/crash; never cancel global AV work.
    unsafe {
        let raw = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        ensure!(
            !raw.is_null(),
            "Cannot create helper process guard: {}",
            std::io::Error::last_os_error()
        );
        let job = OwnedHandle::from_raw_handle(raw);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if let Some(bytes) = memory_limit {
            limits.BasicLimitInformation.LimitFlags |=
                JOB_OBJECT_LIMIT_PROCESS_MEMORY | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
            limits.ProcessMemoryLimit = bytes;
            limits.BasicLimitInformation.ActiveProcessLimit = 1;
        }
        ensure!(
            SetInformationJobObject(
                raw,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32
            ) != 0,
            "Cannot configure helper process guard: {}",
            std::io::Error::last_os_error()
        );
        ensure!(
            AssignProcessToJobObject(raw, child.as_raw_handle()) != 0,
            "Cannot protect helper process lifetime: {}",
            std::io::Error::last_os_error()
        );
        Ok(job)
    }
}
#[cfg(windows)]
fn suspended_thread(child: &Child) -> Result<u32> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::{ERROR_NO_MORE_ITEMS, ERROR_SUCCESS},
        System::{Diagnostics::ProcessSnapshotting::*, Threading::GetCurrentProcess},
    };
    struct Snapshot(HPSS);
    impl Drop for Snapshot {
        fn drop(&mut self) {
            unsafe { PssFreeSnapshot(GetCurrentProcess(), self.0) };
        }
    }
    struct Walk(HPSSWALK);
    impl Drop for Walk {
        fn drop(&mut self) {
            unsafe { PssWalkMarkerFree(self.0) };
        }
    }
    let error = |status: u32| std::io::Error::from_raw_os_error(status as i32);
    // Lists only this child's threads. A system-wide Toolhelp thread snapshot
    // cost 50 ms to 2 s per launch, depending on system load.
    unsafe {
        let mut raw = std::ptr::null_mut();
        let status = PssCaptureSnapshot(child.as_raw_handle(), PSS_CAPTURE_THREADS, 0, &mut raw);
        ensure!(
            status == ERROR_SUCCESS,
            "Cannot inspect the suspended helper: {}",
            error(status)
        );
        let snapshot = Snapshot(raw);
        let mut raw = std::ptr::null_mut();
        let status = PssWalkMarkerCreate(std::ptr::null(), &mut raw);
        ensure!(
            status == ERROR_SUCCESS,
            "Cannot locate the helper thread: {}",
            error(status)
        );
        let walk = Walk(raw);
        let mut id = None;
        loop {
            let mut entry = PSS_THREAD_ENTRY::default();
            match PssWalkSnapshot(
                snapshot.0,
                PSS_WALK_THREADS,
                walk.0,
                (&mut entry as *mut PSS_THREAD_ENTRY).cast(),
                std::mem::size_of::<PSS_THREAD_ENTRY>() as u32,
            ) {
                ERROR_SUCCESS => {}
                ERROR_NO_MORE_ITEMS => break,
                status => {
                    anyhow::bail!("Cannot finish inspecting helper threads: {}", error(status))
                }
            }
            ensure!(
                id.is_none(),
                "Unexpected threads in the suspended helper; refusing to start it"
            );
            id = Some(entry.ThreadId);
        }
        id.context("The suspended helper thread disappeared")
    }
}
#[cfg(windows)]
fn resume_helper(child: &Child) -> Result<()> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::Threading::{
        GetProcessIdOfThread, OpenThread, ResumeThread, THREAD_QUERY_LIMITED_INFORMATION,
        THREAD_SUSPEND_RESUME,
    };
    // std::process retains the process handle but closes CreateProcess's primary
    // thread handle. A newly created suspended process has one initial thread;
    // find only that owned child's thread, after attaching the kill-on-close Job.
    unsafe {
        let raw = OpenThread(
            THREAD_SUSPEND_RESUME | THREAD_QUERY_LIMITED_INFORMATION,
            0,
            suspended_thread(child)?,
        );
        ensure!(
            !raw.is_null(),
            "Cannot open the helper thread: {}",
            std::io::Error::last_os_error()
        );
        let thread = OwnedHandle::from_raw_handle(raw);
        ensure!(
            GetProcessIdOfThread(thread.as_raw_handle()) == child.id(),
            "The helper thread no longer belongs to this process"
        );
        ensure!(
            ResumeThread(thread.as_raw_handle()) == 1,
            "Cannot resume the guarded helper thread"
        );
    }
    Ok(())
}
impl Running {
    /// Attach the suspended child to its kill-on-close Job before any child code runs.
    pub(crate) fn spawn(
        command: &mut Command,
        cancel: &CancellationToken,
        memory_limit: Option<usize>,
    ) -> Result<Self> {
        ensure!(!cancel.is_cancelled(), "Operation cancelled");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, CREATE_SUSPENDED};
            command.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
        }
        let child = command.spawn().context("Cannot start the helper process")?;
        let mut running = Self {
            child,
            #[cfg(windows)]
            tree: None,
        };
        #[cfg(windows)]
        {
            running.tree = Some(process_tree(&running.child, memory_limit)?);
            ensure!(!cancel.is_cancelled(), "Operation cancelled");
            resume_helper(&running.child)?;
        }
        #[cfg(not(windows))]
        let _ = memory_limit;
        Ok(running)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::{
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle},
        process::Stdio,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::WAIT_OBJECT_0,
        System::{
            JobObjects::IsProcessInJob,
            Threading::{
                GetExitCodeProcess, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
            },
        },
    };

    // A child of the unit-test executable, never an arbitrary command.
    #[test]
    fn guarded_child_fixture() {
        let Ok(mode) = std::env::var("IDM_EX_GUARDED_CHILD") else {
            return;
        };
        match mode.as_str() {
            "wait" => std::thread::sleep(Duration::from_secs(120)),
            "tree" => {
                // The Job, not this fixture, owns the descendant's lifetime.
                let descendant = fixture("wait").spawn().unwrap().id();
                let ready = std::env::var_os("IDM_EX_GUARDED_READY").unwrap();
                std::fs::write(ready, descendant.to_string()).unwrap();
                std::thread::sleep(Duration::from_secs(120));
            }
            "limits" => {
                let memory = Vec::<u8>::new().try_reserve_exact(512 << 20).is_ok();
                let process = fixture("wait").spawn().map(|mut c| c.kill()).is_ok();
                std::process::exit(i32::from(memory) | (i32::from(process) << 1));
            }
            _ => panic!("Unknown test mode"),
        }
    }
    fn fixture(mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "child_process::tests::guarded_child_fixture",
                "--nocapture",
            ])
            .env("IDM_EX_GUARDED_CHILD", mode)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }
    fn exits_within(process: RawHandle, timeout: Duration) -> bool {
        unsafe { WaitForSingleObject(process, timeout.as_millis() as u32) == WAIT_OBJECT_0 }
    }

    #[test]
    fn closing_the_job_ends_the_child_and_its_descendants() {
        let temp = tempfile::tempdir().unwrap();
        let ready = temp.path().join("ready");
        let mut command = fixture("tree");
        command.env("IDM_EX_GUARDED_READY", &ready);
        let mut running = Running::spawn(&mut command, &CancellationToken::new(), None).unwrap();
        let job = running.tree.as_ref().unwrap().as_raw_handle();
        let mut member = 0;
        assert_ne!(
            unsafe { IsProcessInJob(running.child.as_raw_handle(), job, &mut member) },
            0
        );
        assert_ne!(member, 0, "The child runs outside its own Job");
        // The resumed child must reach its own code and start a descendant.
        let deadline = Instant::now() + Duration::from_secs(60);
        let descendant = loop {
            if let Some(id) = std::fs::read_to_string(&ready)
                .ok()
                .and_then(|text| text.parse().ok())
            {
                break id;
            }
            assert!(Instant::now() < deadline, "The child did not start");
            std::thread::sleep(Duration::from_millis(10));
        };
        let descendant = unsafe {
            let raw = OpenProcess(PROCESS_SYNCHRONIZE, 0, descendant);
            assert!(!raw.is_null(), "{}", std::io::Error::last_os_error());
            OwnedHandle::from_raw_handle(raw)
        };
        // Closing the only Job handle, as when Download Engine exits or crashes,
        // must end both processes without Running's own kill. They would
        // otherwise sleep for 120 seconds.
        drop(running.tree.take());
        assert!(exits_within(
            running.child.as_raw_handle(),
            Duration::from_secs(30)
        ));
        assert!(exits_within(
            descendant.as_raw_handle(),
            Duration::from_secs(30)
        ));
    }

    #[test]
    fn memory_limit_also_prevents_descendants() {
        for (limit, expected) in [(None, 3), (Some(128 << 20), 0)] {
            let running =
                Running::spawn(&mut fixture("limits"), &CancellationToken::new(), limit).unwrap();
            assert!(exits_within(
                running.child.as_raw_handle(),
                Duration::from_secs(60)
            ));
            let mut code = 0;
            assert_ne!(
                unsafe { GetExitCodeProcess(running.child.as_raw_handle(), &mut code) },
                0
            );
            assert_eq!(code, expected, "memory limit {limit:?}");
        }
    }
}
