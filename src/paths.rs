use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
#[derive(Clone, Debug, serde::Serialize, Deserialize)]
pub struct AppPaths {
    pub data: PathBuf,
    pub downloads: PathBuf,
}
#[derive(Deserialize)]
struct Portable {
    data: PathBuf,
    downloads: PathBuf,
}
/// Selects a separate profile folder, for tests and isolated setups.
pub const HOME_VARIABLE: &str = "DOWNLOAD_ENGINE_HOME";
/// The same setting's name before 2.0, still honored.
pub const LEGACY_HOME_VARIABLE: &str = "IDM_EX_HOME";

/// The profile folder selected through the environment, if any.
pub fn home_override() -> Option<std::ffi::OsString> {
    std::env::var_os(HOME_VARIABLE).or_else(|| std::env::var_os(LEGACY_HOME_VARIABLE))
}

impl AppPaths {
    pub fn discover() -> Result<Self> {
        Self::discover_with(home_override().as_deref(), std::env::current_exe, || {
            let local = PathBuf::from(
                std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is unavailable")?,
            );
            Ok((local, user_downloads()?))
        })
    }

    // Keep OS lookups lazy: an explicit profile or portable marker must work
    // even if the installed user's folders are unavailable. Tests supply these
    // two external lookups without changing process globals or Windows settings.
    fn discover_with(
        home: Option<&std::ffi::OsStr>,
        executable: impl FnOnce() -> std::io::Result<PathBuf>,
        installed_folders: impl FnOnce() -> Result<(PathBuf, PathBuf)>,
    ) -> Result<Self> {
        if let Some(root) = home {
            let root = PathBuf::from(root);
            anyhow::ensure!(
                root.is_absolute(),
                "{HOME_VARIABLE} or {LEGACY_HOME_VARIABLE} must be absolute"
            );
            return Ok(Self {
                data: root.join("data"),
                downloads: root.join("downloads"),
            });
        }
        let exe = executable()?;
        let base = exe.parent().context("Application folder unavailable")?;
        let portable = base.join("portable.json");
        if portable.exists() {
            let p: Portable = serde_json::from_slice(&std::fs::read(portable)?)?;
            return Ok(Self {
                data: base.join(p.data),
                downloads: base.join(p.downloads),
            });
        }
        let (local, downloads) = installed_folders()?;
        Ok(Self::installed(&local, &downloads))
    }
    /// Profiles created before 2.0 keep their data folder and download subfolder
    /// name. Windows supplies the current Downloads root on every discovery;
    /// existing jobs keep their recorded destinations and unfinished data.
    /// A new profile wins once it exists, so an older copy started later cannot
    /// switch this one back.
    fn installed(local_app_data: &Path, user_downloads: &Path) -> Self {
        let data = local_app_data.join("Download Engine");
        let legacy = local_app_data.join("IDM_EX");
        if !data.is_dir() && legacy.join("queue.sqlite3").is_file() {
            return Self {
                data: legacy,
                downloads: user_downloads.join("IDM EX"),
            };
        }
        Self {
            data,
            downloads: user_downloads.join("Download Engine"),
        }
    }
    pub fn prepare(&self) -> Result<()> {
        std::fs::create_dir_all(&self.data)?;
        std::fs::create_dir_all(&self.downloads)?;
        std::fs::create_dir_all(self.data.join("parts"))?;
        Ok(())
    }
    pub fn parts(&self, id: &str) -> Result<PathBuf> {
        crate::model::validate_job_id(id)?;
        Ok(self.data.join("parts").join(id))
    }
}

#[cfg(windows)]
fn user_downloads() -> Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{
        FOLDERID_Downloads, KF_FLAG_DONT_VERIFY, SHGetKnownFolderPath,
    };

    // Resolve the current user's registered location, including redirection.
    // Do not request DEFAULT_PATH or cache this result across discoveries.
    // DONT_VERIFY lets prepare() create a missing folder and avoids probing a
    // disconnected destination during discovery. It does not change Windows.
    let mut value = std::ptr::null_mut();
    let result = unsafe {
        SHGetKnownFolderPath(
            &FOLDERID_Downloads,
            KF_FLAG_DONT_VERIFY as u32,
            std::ptr::null_mut(),
            &mut value,
        )
    };
    let path = if result >= 0 && !value.is_null() {
        // On success Windows owns a terminated UTF-16 buffer. Copy its exact
        // OS string before freeing it, including names not representable in UTF-8.
        let value = windows_core::PWSTR(value);
        Some(PathBuf::from(std::ffi::OsString::from_wide(unsafe {
            value.as_wide()
        })))
    } else {
        None
    };
    // The raw API keeps its output pointer available for cleanup even on failure.
    unsafe { CoTaskMemFree(Some(value.cast())) };
    windows_core::HRESULT(result)
        .ok()
        .context("Windows could not locate the current Downloads folder")?;
    path.filter(|path| path.is_absolute())
        .context("Windows returned an invalid Downloads folder")
}

#[cfg(not(windows))]
fn user_downloads() -> Result<PathBuf> {
    Ok(
        PathBuf::from(std::env::var_os("USERPROFILE").context("USERPROFILE is unavailable")?)
            .join("Downloads"),
    )
}

// Resolve the Windows-owned executable without searching the app folder or PATH.
#[cfg(windows)]
pub fn file_browser() -> Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW;
    let mut buffer = vec![0u16; 32768];
    // The writable UTF-16 buffer is valid for exactly the capacity passed here.
    let length = unsafe { GetWindowsDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
    if length == 0 {
        return Err(std::io::Error::last_os_error()).context("Cannot locate Windows Explorer");
    }
    anyhow::ensure!(
        (length as usize) < buffer.len(),
        "Windows directory path is too long"
    );
    Ok(
        PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length as usize]))
            .join("explorer.exe"),
    )
}
#[cfg(not(windows))]
pub fn file_browser() -> Result<PathBuf> {
    anyhow::bail!("Folder browsing is supported on Windows")
}

// Inno Setup checks this marker while installing/uninstalling an installed copy.
// Queue ownership remains the existing per-data-directory file lock.
#[cfg(windows)]
pub fn installation_guard() -> Result<Option<std::os::windows::io::OwnedHandle>> {
    use std::os::windows::io::{FromRawHandle, OwnedHandle};
    if !std::env::current_exe()?
        .parent()
        .context("Missing application folder")?
        .join("installed.json")
        .is_file()
    {
        return Ok(None);
    }
    let name: Vec<u16> = "Local\\IDM_EX.Installed"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let handle = unsafe {
        windows_sys::Win32::System::Threading::CreateMutexW(std::ptr::null(), 0, name.as_ptr())
    };
    if handle.is_null() {
        return Err(std::io::Error::last_os_error())
            .context("Cannot protect the running installation");
    }
    // The new handle is owned here and stays open throughout the process lifecycle.
    Ok(Some(unsafe { OwnedHandle::from_raw_handle(handle) }))
}
#[cfg(not(windows))]
pub fn installation_guard() -> Result<Option<()>> {
    Ok(None)
}

// Explicit user action only: let Windows open a completed local file using its
// normal association and security prompts. Never invoke a command interpreter.
#[cfg(windows)]
pub fn open_download(path: &std::path::Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    anyhow::ensure!(path.is_absolute(), "The saved file path must be absolute");
    anyhow::ensure!(path.is_file(), "The saved file no longer exists");
    let mut file: Vec<u16> = path.as_os_str().encode_wide().collect();
    anyhow::ensure!(
        !file.contains(&0),
        "The saved file path contains a null character"
    );
    file.push(0);
    let verb: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
    // The terminated strings remain alive for the call. ShellExecuteW returns
    // a status value, not an owned handle; Windows controls any security UI.
    let result = unsafe {
        windows_sys::Win32::UI::Shell::ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        )
    } as isize;
    anyhow::ensure!(
        result > 32,
        "Windows could not open this file (code {result})"
    );
    Ok(())
}
#[cfg(not(windows))]
pub fn open_download(_path: &std::path::Path) -> Result<()> {
    anyhow::bail!("Opening downloads is supported on Windows")
}

#[cfg(windows)]
pub fn open_download_with(path: &std::path::Path) -> Result<()> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW;
    anyhow::ensure!(path.is_absolute(), "The saved file path must be absolute");
    anyhow::ensure!(path.is_file(), "The saved file no longer exists");
    let mut buffer = vec![0u16; 32768];
    let length = unsafe { GetWindowsDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
    if length == 0 {
        return Err(std::io::Error::last_os_error()).context("Cannot locate Windows directory");
    }
    let rundll32 = PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length as usize]))
        .join("System32")
        .join("rundll32.exe");
    std::process::Command::new(rundll32)
        .arg("shell32.dll,OpenAs_RunDLL")
        .arg(path)
        .spawn()
        .context("Windows could not open the application chooser")?;
    Ok(())
}
#[cfg(not(windows))]
pub fn open_download_with(_path: &std::path::Path) -> Result<()> {
    anyhow::bail!("Opening downloads is supported on Windows")
}

#[cfg(all(test, windows))]
#[path = "paths/discovery_tests.rs"]
mod discovery_tests;

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn opening_downloads_rejects_relative_missing_and_directory_paths() {
        assert!(
            open_download(std::path::Path::new("relative.exe"))
                .unwrap_err()
                .to_string()
                .contains("absolute")
        );
        let root = tempfile::tempdir().unwrap();
        for path in [root.path().to_path_buf(), root.path().join("missing.exe")] {
            assert!(
                open_download(&path)
                    .unwrap_err()
                    .to_string()
                    .contains("no longer exists")
            );
        }
    }
    #[test]
    fn existing_profiles_keep_their_folders_and_new_profiles_use_new_names() {
        let root = tempfile::tempdir().unwrap();
        let (local, downloads) = (root.path().join("Local"), root.path().join("Downloads"));
        let fresh = AppPaths::installed(&local, &downloads);
        assert_eq!(fresh.data, local.join("Download Engine"));
        assert_eq!(fresh.downloads, downloads.join("Download Engine"));

        // Browser setup keeps a lock file in the old folder; that alone is not a profile.
        std::fs::create_dir_all(local.join("IDM_EX")).unwrap();
        std::fs::write(local.join("IDM_EX/browser-registration.lock"), b"").unwrap();
        assert_eq!(AppPaths::installed(&local, &downloads).data, fresh.data);

        std::fs::write(local.join("IDM_EX/queue.sqlite3"), b"").unwrap();
        let existing = AppPaths::installed(&local, &downloads);
        assert_eq!(existing.data, local.join("IDM_EX"));
        assert_eq!(existing.downloads, downloads.join("IDM EX"));

        // Once a new profile exists, an old one created later never replaces it.
        std::fs::create_dir_all(local.join("Download Engine")).unwrap();
        assert_eq!(AppPaths::installed(&local, &downloads).data, fresh.data);
    }
}
