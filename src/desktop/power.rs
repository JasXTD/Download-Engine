//! Only the explicitly armed desktop completion action may shut down, sleep or
//! hibernate. Test and screenshot builds cannot request a real power action or
//! play a sound.
use download_engine::completion::{PowerStates, Sound};
use download_engine::i18n::tr;
#[cfg(any(test, feature = "screenshot", not(windows)))]
pub(super) fn shutdown() -> anyhow::Result<()> {
    anyhow::bail!(tr(
        "Computer shutdown is unavailable in test builds or on this platform"
    ))
}

#[cfg(all(windows, not(test), not(feature = "screenshot")))]
pub(super) fn shutdown() -> anyhow::Result<()> {
    use anyhow::{Context, ensure};
    use std::os::windows::{ffi::OsStringExt, process::CommandExt};
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
    let mut buffer = vec![0u16; 32768];
    // A Windows-owned path avoids searching PATH or the app's writable directory.
    let length = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
    if length == 0 {
        return Err(std::io::Error::last_os_error()).context(tr("Cannot locate Windows shutdown"));
    }
    ensure!(
        (length as usize) < buffer.len(),
        tr("Windows system directory path is too long")
    );
    let executable =
        std::path::PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length as usize]))
            .join("shutdown.exe");
    // Windows implicitly forces applications closed for /t > 0. Our cancellable
    // countdown runs inside Download Engine; /t 0 without /f lets Windows protect
    // unsaved work in other applications. Never add /f here.
    let output = std::process::Command::new(executable)
        .args(["/s", "/t", "0"])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .output()
        .context(tr("Could not request Windows shutdown"))?;
    ensure!(
        output.status.success(),
        "Windows rejected the shutdown request ({}). Save your work and shut down manually.",
        output.status
    );
    Ok(())
}

/// Sleep, or hibernate when `hibernate` is set. Windows returns only after the
/// computer wakes, so call this off the UI thread.
#[cfg(any(test, feature = "screenshot", not(windows)))]
pub(super) fn suspend(_hibernate: bool) -> anyhow::Result<()> {
    anyhow::bail!(tr(
        "Sleep and hibernation are unavailable in test builds or on this platform"
    ))
}

#[cfg(all(windows, not(test), not(feature = "screenshot")))]
pub(super) fn suspend(hibernate: bool) -> anyhow::Result<()> {
    use anyhow::Context;
    use windows_sys::Win32::System::Power::SetSuspendState;
    let _privilege = ShutdownPrivilege::enable()?;
    // Never force: other applications keep their say, and wake events such as
    // scheduled tasks stay enabled.
    if unsafe { SetSuspendState(hibernate, false, false) } {
        return Ok(());
    }
    Err(std::io::Error::last_os_error()).context(if hibernate {
        tr("Windows did not hibernate this computer")
    } else {
        tr("Windows did not put this computer to sleep")
    })
}

/// SetSuspendState requires SeShutdownPrivilege enabled in the process token.
/// Dropping this restores the previous state and closes the token.
#[cfg(all(windows, not(test), not(feature = "screenshot")))]
struct ShutdownPrivilege {
    token: windows_sys::Win32::Foundation::HANDLE,
    previous: windows_sys::Win32::Security::TOKEN_PRIVILEGES,
    adjusted: bool,
}

#[cfg(all(windows, not(test), not(feature = "screenshot")))]
impl ShutdownPrivilege {
    fn enable() -> anyhow::Result<Self> {
        use anyhow::Context;
        use windows_sys::Win32::{
            Foundation::{ERROR_NOT_ALL_ASSIGNED, GetLastError},
            Security::{
                AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW,
                SE_PRIVILEGE_ENABLED, SE_SHUTDOWN_NAME, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES,
                TOKEN_QUERY,
            },
            System::Threading::{GetCurrentProcess, OpenProcessToken},
        };
        let failed = || {
            Err(std::io::Error::last_os_error())
                .context(tr("Could not request sleep or hibernation"))
        };
        let mut token = std::ptr::null_mut();
        if unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
                &mut token,
            )
        } == 0
        {
            return failed();
        }
        let mut privilege = Self {
            token,
            previous: unsafe { std::mem::zeroed() },
            adjusted: false,
        };
        let mut wanted = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: unsafe { std::mem::zeroed() },
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        if unsafe {
            LookupPrivilegeValueW(
                std::ptr::null(),
                SE_SHUTDOWN_NAME,
                &mut wanted.Privileges[0].Luid,
            )
        } == 0
        {
            return failed();
        }
        let mut length = 0;
        if unsafe {
            AdjustTokenPrivileges(
                token,
                0,
                &wanted,
                size_of::<TOKEN_PRIVILEGES>() as u32,
                &mut privilege.previous,
                &mut length,
            )
        } == 0
        {
            return failed();
        }
        privilege.adjusted = true;
        // AdjustTokenPrivileges also succeeds when the account lacks the privilege.
        if unsafe { GetLastError() } == ERROR_NOT_ALL_ASSIGNED {
            anyhow::bail!(tr(
                "This Windows account is not allowed to put the computer to sleep or hibernate it."
            ));
        }
        Ok(privilege)
    }
}

#[cfg(all(windows, not(test), not(feature = "screenshot")))]
impl Drop for ShutdownPrivilege {
    fn drop(&mut self) {
        use windows_sys::Win32::{Foundation::CloseHandle, Security::AdjustTokenPrivileges};
        unsafe {
            if self.adjusted {
                AdjustTokenPrivileges(
                    self.token,
                    0,
                    &self.previous,
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                );
            }
            CloseHandle(self.token);
        }
    }
}

/// What Windows lets an app request. Read-only, so every build may ask.
pub(super) fn power_states() -> PowerStates {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Power::{GetPwrCapabilities, SYSTEM_POWER_CAPABILITIES};
        let mut capabilities: SYSTEM_POWER_CAPABILITIES = unsafe { std::mem::zeroed() };
        if unsafe { GetPwrCapabilities(&mut capabilities) } {
            return PowerStates {
                sleep: capabilities.SystemS1 || capabilities.SystemS2 || capabilities.SystemS3,
                hibernate: capabilities.SystemS4 && capabilities.HiberFilePresent,
            };
        }
    }
    // Without an answer, offer neither; the choices explain why.
    PowerStates::default()
}

#[cfg(test)]
thread_local! {
    /// Sounds a real build would have played on this test thread.
    pub(super) static PLAYED: std::cell::RefCell<Vec<Sound>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(any(test, feature = "screenshot", not(windows)))]
pub(super) fn play(sound: Sound) {
    #[cfg(test)]
    PLAYED.with(|played| played.borrow_mut().push(sound));
    #[cfg(not(test))]
    let _ = sound;
}

#[cfg(all(windows, not(test), not(feature = "screenshot")))]
pub(super) fn play(sound: Sound) {
    use windows_sys::{
        Win32::{
            Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC, SND_NODEFAULT, SND_SYSTEM},
            UI::Shell::{
                QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_QUIET_TIME, QUNS_RUNNING_D3D_FULL_SCREEN,
                SHQueryUserNotificationState,
            },
        },
        w,
    };
    // Like the tray notifications, stay quiet over full-screen apps,
    // presentations and Windows quiet time.
    let mut state = 0;
    if unsafe { SHQueryUserNotificationState(&mut state) } >= 0
        && matches!(
            state,
            QUNS_BUSY | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE | QUNS_QUIET_TIME
        )
    {
        return;
    }
    let alias = match sound {
        Sound::Complete => w!("Notification.Default"),
        Sound::Attention => w!("SystemExclamation"),
    };
    // System sounds follow Windows Sound settings: an event set to (None) stays
    // silent, and SND_NODEFAULT never falls back to a beep. A false result only
    // means that nothing played.
    unsafe {
        PlaySoundW(
            alias,
            std::ptr::null_mut(),
            SND_ALIAS | SND_ASYNC | SND_NODEFAULT | SND_SYSTEM,
        )
    };
}
