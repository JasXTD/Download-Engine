//! Per-user Default Apps registration. Never writes UserChoice or replaces an
//! existing .torrent/magnet default. See Microsoft's Default Programs contract.
use anyhow::{Context, Result, ensure};
use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq)]
pub enum Status {
    NotRegistered,
    ThisCopy,
    OtherCopy(PathBuf),
}
#[derive(Clone, Copy)]
pub enum Action {
    Refresh,
    Register,
    Remove,
    Defaults,
}
pub fn apply(action: Action) -> Result<Status> {
    let registration = Registration::new(std::env::current_exe()?);
    match action {
        Action::Refresh => {}
        Action::Register => registration.register()?,
        Action::Remove => registration.remove()?,
        Action::Defaults => open_defaults()?,
    }
    registration.status()
}

struct Registration {
    prefix: String,
    exe: PathBuf,
}
impl Registration {
    fn new(exe: PathBuf) -> Self {
        Self {
            prefix: "Software".into(),
            exe,
        }
    }
    fn classes(&self) -> String {
        format!("{}\\Classes", self.prefix)
    }
    fn app(&self) -> String {
        format!("{}\\DownloadEngine\\TorrentHandlers", self.prefix)
    }
    fn capabilities(&self) -> String {
        format!("{}\\Capabilities", self.app())
    }
    fn registered(&self) -> String {
        format!("{}\\RegisteredApplications", self.prefix)
    }
    fn command(&self) -> Result<String> {
        let path = self
            .exe
            .to_str()
            .context("The application path is not valid Unicode")?;
        ensure!(
            self.exe.is_absolute() && !path.contains(['"', '\0', '\r', '\n']),
            "Invalid application path"
        );
        Ok(format!("\"{path}\" --open \"%1\""))
    }
    fn owns(&self) -> Result<bool> {
        Ok(registry::read(&self.app(), "OwnerExecutable")?
            .is_some_and(|v| v.eq_ignore_ascii_case(&self.exe.to_string_lossy())))
    }
    fn status(&self) -> Result<Status> {
        let Some(owner) = registry::read(&self.app(), "OwnerExecutable")? else {
            return Ok(Status::NotRegistered);
        };
        if !self.owns()? {
            return Ok(Status::OtherCopy(PathBuf::from(owner)));
        }
        let command = self.command()?;
        for id in ["DownloadEngine.Torrent", "DownloadEngine.Magnet"] {
            if registry::read(
                &format!("{}\\{id}\\shell\\open\\command", self.classes()),
                "",
            )?
            .as_deref()
                != Some(command.as_str())
            {
                return Ok(Status::NotRegistered);
            }
        }
        if registry::read(&self.registered(), "Download Engine")?.as_deref()
            != Some(self.capabilities().as_str())
        {
            return Ok(Status::NotRegistered);
        }
        for (key, name, expected) in [
            (
                format!("{}\\FileAssociations", self.capabilities()),
                ".torrent",
                "DownloadEngine.Torrent",
            ),
            (
                format!("{}\\UrlAssociations", self.capabilities()),
                "magnet",
                "DownloadEngine.Magnet",
            ),
            (
                format!("{}\\.torrent\\OpenWithProgids", self.classes()),
                "DownloadEngine.Torrent",
                "",
            ),
            (
                format!("{}\\DownloadEngine.Magnet", self.classes()),
                "URL Protocol",
                "",
            ),
        ] {
            if registry::read(&key, name)?.as_deref() != Some(expected) {
                return Ok(Status::NotRegistered);
            }
        }
        Ok(Status::ThisCopy)
    }
    fn register(&self) -> Result<()> {
        let command = self.command()?;
        registry::set(&self.app(), "OwnerExecutable", &self.exe.to_string_lossy())?;
        for (id, title) in [
            ("DownloadEngine.Torrent", "Torrent file"),
            ("DownloadEngine.Magnet", "Magnet link"),
        ] {
            let key = format!("{}\\{id}", self.classes());
            registry::set(&key, "", title)?;
            if id.ends_with("Magnet") {
                registry::set(&key, "URL Protocol", "")?;
            }
            registry::set(
                &format!("{key}\\DefaultIcon"),
                "",
                &format!("\"{}\",0", self.exe.display()),
            )?;
            registry::set(&format!("{key}\\shell\\open\\command"), "", &command)?;
        }
        registry::set(
            &format!("{}\\.torrent\\OpenWithProgids", self.classes()),
            "DownloadEngine.Torrent",
            "",
        )?;
        let cap = self.capabilities();
        registry::set(&cap, "ApplicationName", "Download Engine")?;
        registry::set(
            &cap,
            "ApplicationDescription",
            "Review torrent files and magnet links before downloading.",
        )?;
        registry::set(
            &cap,
            "ApplicationIcon",
            &format!("\"{}\",0", self.exe.display()),
        )?;
        registry::set(
            &format!("{cap}\\FileAssociations"),
            ".torrent",
            "DownloadEngine.Torrent",
        )?;
        registry::set(
            &format!("{cap}\\UrlAssociations"),
            "magnet",
            "DownloadEngine.Magnet",
        )?;
        registry::set(&self.registered(), "Download Engine", &cap)?;
        registry::notify();
        Ok(())
    }
    fn remove(&self) -> Result<()> {
        if !self.owns()? {
            return Ok(());
        }
        let command = self.command()?;
        for id in ["DownloadEngine.Torrent", "DownloadEngine.Magnet"] {
            let key = format!("{}\\{id}", self.classes());
            if registry::read(&format!("{key}\\shell\\open\\command"), "")?.as_deref()
                == Some(command.as_str())
            {
                registry::delete_tree(&key)?;
            }
        }
        registry::delete_value(
            &format!("{}\\.torrent\\OpenWithProgids", self.classes()),
            "DownloadEngine.Torrent",
        )?;
        if registry::read(&self.registered(), "Download Engine")?.as_deref()
            == Some(self.capabilities().as_str())
        {
            registry::delete_value(&self.registered(), "Download Engine")?;
        }
        registry::delete_tree(&self.app())?;
        registry::notify();
        Ok(())
    }
}

#[cfg(windows)]
fn open_defaults() -> Result<()> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    let uri: Vec<u16> = "ms-settings:defaultapps?registeredAppUser=Download%20Engine"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            std::ptr::null(),
            uri.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        )
    };
    ensure!(
        result as isize > 32,
        "Windows Default Apps could not be opened"
    );
    Ok(())
}
#[cfg(not(windows))]
fn open_defaults() -> Result<()> {
    anyhow::bail!("Windows integration is only available on Windows")
}

#[cfg(windows)]
mod registry {
    use super::*;
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::*,
    };
    fn wide(s: &str) -> Result<Vec<u16>> {
        ensure!(!s.contains('\0'), "Invalid registry string");
        Ok(s.encode_utf16().chain(Some(0)).collect())
    }
    fn check(code: u32) -> Result<()> {
        if code == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(std::io::Error::from_raw_os_error(code as i32).into())
        }
    }
    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }
    pub fn read(path: &str, name: &str) -> Result<Option<String>> {
        let (path, name) = (wide(path)?, wide(name)?);
        let mut size = 0;
        let code = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                path.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if code == ERROR_FILE_NOT_FOUND || code == ERROR_PATH_NOT_FOUND {
            return Ok(None);
        }
        check(code)?;
        ensure!(
            size <= 65536 && size % 2 == 0,
            "Invalid Windows registration value"
        );
        let mut buffer = vec![0u16; (size as usize / 2).max(1)];
        check(unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                path.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        })?;
        while buffer.last() == Some(&0) {
            buffer.pop();
        }
        Ok(Some(String::from_utf16(&buffer)?))
    }
    pub fn set(path: &str, name: &str, value: &str) -> Result<()> {
        let (path, name, value) = (wide(path)?, wide(name)?, wide(value)?);
        let mut key = std::ptr::null_mut();
        check(unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                path.as_ptr(),
                0,
                std::ptr::null(),
                0,
                KEY_SET_VALUE,
                std::ptr::null(),
                &mut key,
                std::ptr::null_mut(),
            )
        })?;
        let key = Key(key);
        check(unsafe {
            RegSetValueExW(
                key.0,
                name.as_ptr(),
                0,
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * 2) as u32,
            )
        })
    }
    pub fn delete_value(path: &str, name: &str) -> Result<()> {
        let (path, name) = (wide(path)?, wide(name)?);
        let code = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, path.as_ptr(), name.as_ptr()) };
        if code == ERROR_FILE_NOT_FOUND || code == ERROR_PATH_NOT_FOUND {
            Ok(())
        } else {
            check(code)
        }
    }
    pub fn delete_tree(path: &str) -> Result<()> {
        let path = wide(path)?;
        let code = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, path.as_ptr()) };
        if code == ERROR_FILE_NOT_FOUND || code == ERROR_PATH_NOT_FOUND {
            Ok(())
        } else {
            check(code)
        }
    }
    pub fn notify() {
        #[cfg(not(test))]
        unsafe {
            use windows_sys::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
            SHChangeNotify(
                SHCNE_ASSOCCHANGED as i32,
                SHCNF_IDLIST,
                std::ptr::null(),
                std::ptr::null(),
            );
        }
    }
}
#[cfg(not(windows))]
mod registry {
    use super::*;
    pub fn read(_: &str, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
    pub fn set(_: &str, _: &str, _: &str) -> Result<()> {
        anyhow::bail!("Windows integration is only available on Windows")
    }
    pub fn delete_value(_: &str, _: &str) -> Result<()> {
        Ok(())
    }
    pub fn delete_tree(_: &str) -> Result<()> {
        Ok(())
    }
    pub fn notify() {}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn handler_quotes_executable_and_the_single_external_input() {
        let exe = std::env::current_dir()
            .unwrap()
            .join("Example folder")
            .join("de.exe");
        assert_eq!(
            Registration::new(exe.clone()).command().unwrap(),
            format!("\"{}\" --open \"%1\"", exe.display())
        );
        assert!(
            Registration::new(PathBuf::from("de.exe"))
                .command()
                .is_err()
        );
    }
    #[cfg(windows)]
    #[test]
    fn isolated_registration_preserves_defaults_and_other_copy_ownership() {
        let prefix = format!("Software\\DownloadEngineTests\\{}", uuid::Uuid::new_v4());
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                registry::delete_tree(&self.0).unwrap();
            }
        }
        let _cleanup = Cleanup(prefix.clone());
        let root = std::env::current_dir().unwrap();
        let first = Registration {
            prefix: prefix.clone(),
            exe: root.join("First copy").join("de.exe"),
        };
        let second = Registration {
            prefix,
            exe: root.join("Other copy").join("de.exe"),
        };
        let torrent = format!("{}\\.torrent", first.classes());
        let magnet = format!("{}\\magnet", first.classes());
        registry::set(&torrent, "", "Existing.Torrent").unwrap();
        registry::set(&magnet, "", "Existing.Magnet").unwrap();
        assert_eq!(first.status().unwrap(), Status::NotRegistered);
        first.register().unwrap();
        assert_eq!(first.status().unwrap(), Status::ThisCopy);
        let protocols = format!("{}\\UrlAssociations", first.capabilities());
        registry::delete_value(&protocols, "magnet").unwrap();
        assert_eq!(first.status().unwrap(), Status::NotRegistered);
        first.register().unwrap();
        assert_eq!(first.status().unwrap(), Status::ThisCopy);
        assert_eq!(
            second.status().unwrap(),
            Status::OtherCopy(first.exe.clone())
        );
        second.remove().unwrap();
        assert_eq!(first.status().unwrap(), Status::ThisCopy);
        second.register().unwrap();
        first.remove().unwrap();
        assert_eq!(second.status().unwrap(), Status::ThisCopy);
        second.remove().unwrap();
        assert_eq!(second.status().unwrap(), Status::NotRegistered);
        assert_eq!(
            registry::read(&torrent, "").unwrap().as_deref(),
            Some("Existing.Torrent")
        );
        assert_eq!(
            registry::read(&magnet, "").unwrap().as_deref(),
            Some("Existing.Magnet")
        );
    }
}
