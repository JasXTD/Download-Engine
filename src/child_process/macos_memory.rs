//! macOS maps large shared regions before exec, so a small RLIMIT_AS cannot be
//! applied to the forked app. Watch owned helpers' physical memory instead.
//! This is a sampled guard, not an allocation-level limit; Lua also has its own
//! allocator cap. The owner MUST join this guard before reaping the child PID.
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicI32, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

pub(super) struct Guard {
    stop: Option<mpsc::Sender<()>>,
    worker: Option<JoinHandle<()>>,
    failure: Arc<AtomicI32>,
}
impl Guard {
    pub(super) fn new(pid: u32, limit: usize) -> io::Result<Self> {
        let (stop, stopped) = mpsc::channel();
        let failure = Arc::new(AtomicI32::new(0));
        let result = Arc::clone(&failure);
        let worker = thread::Builder::new()
            .name("helper-memory".into())
            .spawn(move || {
                loop {
                    let mut usage: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
                    let status = unsafe {
                        libc::proc_pid_rusage(
                            pid as i32,
                            libc::RUSAGE_INFO_V2,
                            std::ptr::from_mut(&mut usage).cast(),
                        )
                    };
                    let error = if status == 0 {
                        if usage.ri_phys_footprint.max(usage.ri_resident_size) > limit as u64 {
                            -1
                        } else {
                            0
                        }
                    } else {
                        let error = io::Error::last_os_error()
                            .raw_os_error()
                            .unwrap_or(libc::EIO);
                        if error == libc::ESRCH {
                            break;
                        } // Exited; the owner still reserves its PID.
                        error
                    };
                    if error != 0 {
                        result.store(error, Ordering::Release);
                        // This child established its own process group before exec.
                        unsafe {
                            libc::kill(-(pid as i32), libc::SIGKILL);
                        }
                        break;
                    }
                    match stopped.recv_timeout(Duration::from_millis(25)) {
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        _ => break,
                    }
                }
            })?;
        Ok(Self {
            stop: Some(stop),
            worker: Some(worker),
            failure,
        })
    }
    pub(super) fn error(&self) -> Option<io::Error> {
        match self.failure.load(Ordering::Acquire) {
            0 => None,
            -1 => Some(io::Error::other("Helper exceeded its memory limit")),
            error => Some(io::Error::other(format!(
                "Cannot monitor helper memory: {}",
                io::Error::from_raw_os_error(error)
            ))),
        }
    }
    pub(super) fn finish(&mut self) -> Option<io::Error> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                self.failure.store(libc::EIO, Ordering::Release);
            }
        }
        self.error()
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}
