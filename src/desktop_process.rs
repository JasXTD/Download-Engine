//! Launch the long-lived desktop without inheriting browser or native-host handles.
use anyhow::{Context, Result};
use download_engine::paths::AppPaths;
use std::fs::OpenOptions;
use std::path::Path;

#[cfg(windows)]
pub struct DesktopProcess(std::os::windows::io::OwnedHandle);
#[cfg(not(windows))]
pub struct DesktopProcess(std::process::Child);

#[cfg(windows)]
pub fn launch(executable: &Path) -> Result<DesktopProcess> {
    use std::os::windows::{ffi::OsStrExt, io::FromRawHandle};
    use windows_sys::Win32::System::Threading::{
        CREATE_BREAKAWAY_FROM_JOB, CREATE_NO_WINDOW, CreateProcessW, PROCESS_INFORMATION,
        STARTUPINFOW,
    };
    anyhow::ensure!(
        executable.is_absolute(),
        "Desktop executable path must be absolute"
    );
    let path: Vec<u16> = executable.as_os_str().encode_wide().collect();
    anyhow::ensure!(
        !path.contains(&0) && !path.contains(&34),
        "Invalid desktop executable path"
    );
    // lpApplicationName selects the exact executable. Quote argv[0] for spaces;
    // no arguments are accepted and the mutable command buffer stays alive.
    let mut command: Vec<u16> = std::iter::once(34)
        .chain(path.iter().copied())
        .chain([34, 0])
        .collect();
    let application: Vec<u16> = path.into_iter().chain([0]).collect();
    let startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut information = PROCESS_INFORMATION::default();
    // Clearing only stdin/stdout/stderr inheritance is insufficient: browsers
    // may also pass auxiliary inheritable pipes. The GUI needs none of them.
    // It also must survive Firefox closing the native messaging job object.
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB,
            std::ptr::null(),
            std::ptr::null(),
            &startup,
            &mut information,
        )
    };
    if created == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // Successful CreateProcess transfers these two handles to us. Closing them
    // never terminates the child. Only the process handle is needed for polling.
    let process =
        unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(information.hProcess) };
    let thread = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(information.hThread) };
    drop(thread);
    Ok(DesktopProcess(process))
}

#[cfg(windows)]
impl DesktopProcess {
    pub fn try_wait(&mut self) -> Result<Option<String>> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::{
            Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
            System::Threading::{GetExitCodeProcess, WaitForSingleObject},
        };
        // The owned handle remains valid during both nonblocking calls.
        match unsafe { WaitForSingleObject(self.0.as_raw_handle(), 0) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let mut code = 0;
                if unsafe { GetExitCodeProcess(self.0.as_raw_handle(), &mut code) } == 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
                Ok(Some(code.to_string()))
            }
            _ => Err(std::io::Error::last_os_error().into()),
        }
    }
}

#[cfg(not(windows))]
pub fn launch(executable: &Path) -> Result<DesktopProcess> {
    use std::process::{Command, Stdio};
    Ok(DesktopProcess(
        Command::new(executable)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?,
    ))
}
#[cfg(not(windows))]
impl DesktopProcess {
    pub fn try_wait(&mut self) -> Result<Option<String>> {
        Ok(self.0.try_wait()?.map(|status| status.to_string()))
    }
}

pub fn ensure_running(paths: &AppPaths) -> Result<()> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(paths.data.join("desktop.lock"))?;
    match fs2::FileExt::try_lock_exclusive(&file) {
        Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => return Ok(()),
        Err(e) => return Err(e.into()),
        Ok(()) => fs2::FileExt::unlock(&file)?,
    }
    let executable = std::env::current_exe()?
        .parent()
        .context("Missing application folder")?
        .join("de.exe");
    let mut child = launch(&executable).context("Desktop application could not start")?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match fs2::FileExt::try_lock_exclusive(&file) {
            Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                return Ok(());
            }
            Err(e) => return Err(e.into()),
            Ok(()) => fs2::FileExt::unlock(&file)?,
        }
        if let Some(status) = child.try_wait()? {
            anyhow::bail!("Desktop application exited before it was ready ({status})");
        }
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "Desktop application did not become ready"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
