//! Per-user native-host registration, shared by the desktop and installer helper.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub const HOST_NAME: &str = "com.xetro.downloadengine";
pub const FIREFOX_ID: &str = "download-engine@xetro";
// Separate manifests keep older extensions working while the desktop and
// extension are upgraded independently. Never allow both Firefox IDs in one host.
const LEGACY_HOST_NAME: &str = "com.idmex.downloader";
const LEGACY_FIREFOX_ID: &str = "idm-ex@local";
const HOST_PROGRAM: &str = "de-host.exe";
/// The host's file name before 2.0; manifests written by those releases name it.
const LEGACY_HOST_PROGRAM: &str = "idm-ex-host.exe";
const HOST_DESCRIPTION: &str = "Download Engine download queue";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Browser {
    #[default]
    Chrome,
    Edge,
    Brave,
    Firefox,
}
impl Browser {
    pub const ALL: [Self; 4] = [Self::Chrome, Self::Edge, Self::Brave, Self::Firefox];
    pub fn label(self) -> &'static str {
        match self {
            Self::Chrome => "Chrome",
            Self::Edge => "Edge",
            Self::Brave => "Brave",
            Self::Firefox => "Firefox",
        }
    }
    pub fn slug(self) -> &'static str {
        match self {
            Self::Chrome => "chrome",
            Self::Edge => "edge",
            Self::Brave => "brave",
            Self::Firefox => "firefox",
        }
    }
    fn registry_path(self) -> &'static str {
        match self {
            Self::Chrome => r"Google\Chrome",
            Self::Edge => r"Microsoft\Edge",
            // Brave has no key of its own on Windows. Like other non-Google Chromium
            // builds it checks the Chromium key before Chrome's, so registering here
            // keeps Brave's extension ID separate from Chrome's. Those other browsers
            // also read this key.
            Self::Brave => r"Chromium",
            Self::Firefox => r"Mozilla",
        }
    }
    pub fn parse(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|b| b.slug() == value)
            .context("Choose chrome, edge, brave or firefox")
    }
    pub fn extension_id(self, value: &str) -> Result<String> {
        if self == Self::Firefox {
            ensure!(
                value.is_empty() || value == FIREFOX_ID,
                "Unexpected Firefox extension ID"
            );
            return Ok(FIREFOX_ID.into());
        }
        let value = value.trim();
        ensure!(
            value.len() == 32 && value.bytes().all(|b| (b'a'..=b'p').contains(&b)),
            "Paste the 32-letter extension ID shown on the browser's Extensions page"
        );
        Ok(value.into())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RegistrationStatus {
    Missing,
    Registered { extension_id: String },
    OtherCopy { manifest: PathBuf },
    NeedsRepair { message: String },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostManifest {
    name: String,
    description: String,
    path: PathBuf,
    #[serde(rename = "type")]
    transport: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    allowed_origins: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    allowed_extensions: Vec<String>,
}

impl HostManifest {
    fn extension_id(&self, browser: Browser, name: &str, firefox_id: &str) -> Result<String> {
        ensure!(
            self.name == name && self.transport == "stdio",
            "Unexpected native host identity"
        );
        if browser == Browser::Firefox {
            ensure!(
                self.allowed_origins.is_empty() && self.allowed_extensions == [firefox_id],
                "Unexpected extension permissions"
            );
            return Ok(firefox_id.into());
        }
        ensure!(
            self.allowed_extensions.is_empty() && self.allowed_origins.len() == 1,
            "Unexpected extension permissions"
        );
        let id = self.allowed_origins[0]
            .strip_prefix("chrome-extension://")
            .and_then(|value| value.strip_suffix('/'))
            .context("Invalid extension origin")?;
        browser.extension_id(id)
    }
}

struct RegistrationChange {
    path: PathBuf,
    key: String,
    manifest: HostManifest,
    previous_paths: Vec<Option<PathBuf>>,
    previous_manifest: Option<Vec<u8>>,
}

pub struct BrowserSetup {
    root: PathBuf,
    // Every installed/portable copy uses the same per-user registration lock.
    lock_directory: PathBuf,
    registry_prefix: String,
}
impl BrowserSetup {
    pub fn discover() -> Result<Self> {
        let root = std::env::current_exe()?
            .parent()
            .context("Missing application folder")?
            .to_owned();
        let lock_directory =
            PathBuf::from(std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is unavailable")?)
                .join("IDM_EX");
        Ok(Self {
            root,
            lock_directory,
            registry_prefix: "Software".into(),
        })
    }
    pub fn extension_folder(&self, browser: Browser) -> PathBuf {
        self.root.join("extensions").join(browser.slug())
    }
    /// The ID a Chromium browser gives this copy's extension folder when it is
    /// loaded unpacked. Firefox uses its built-in ID instead.
    pub fn expected_extension_id(&self, browser: Browser) -> Option<String> {
        let folder = self.extension_folder(browser);
        (browser != Browser::Firefox && folder.join("manifest.json").is_file())
            .then(|| unpacked_extension_id(&folder))
    }
    fn manifest_path(&self, browser: Browser) -> PathBuf {
        self.root
            .join(format!("native-download-engine-{}.json", browser.slug()))
    }
    fn legacy_manifest_path(&self, browser: Browser) -> PathBuf {
        self.root.join(format!("native-{}.json", browser.slug()))
    }
    fn registry_key(&self, browser: Browser) -> String {
        format!(
            r"{}\{}\NativeMessagingHosts\{HOST_NAME}",
            self.registry_prefix,
            browser.registry_path()
        )
    }
    fn legacy_registry_key(&self, browser: Browser) -> String {
        format!(
            r"{}\{}\NativeMessagingHosts\{LEGACY_HOST_NAME}",
            self.registry_prefix,
            browser.registry_path()
        )
    }
    fn lock(&self) -> Result<fs::File> {
        fs::create_dir_all(&self.lock_directory)?;
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.lock_directory.join("browser-registration.lock"))?;
        fs2::FileExt::try_lock_exclusive(&file)
            .context("Browser setup is already running. Try again when it finishes")?;
        Ok(file)
    }
    fn registered_paths(&self, browser: Browser) -> Result<Vec<Option<PathBuf>>> {
        self.paths_at(&self.registry_key(browser))
    }
    fn paths_at(&self, key: &str) -> Result<Vec<Option<PathBuf>>> {
        registry::VIEWS
            .iter()
            .map(|&view| registry::read(key, view))
            .collect()
    }
    pub fn status(&self, browser: Browser) -> Result<RegistrationStatus> {
        let expected = self.manifest_path(browser);
        let paths = self.registered_paths(browser)?;
        if let Some(other) = paths.iter().flatten().find(|p| !same_path(p, &expected)) {
            return Ok(RegistrationStatus::OtherCopy {
                manifest: other.clone(),
            });
        }
        let legacy_expected = self.legacy_manifest_path(browser);
        let legacy_paths = self.paths_at(&self.legacy_registry_key(browser))?;
        if let Some(other) = legacy_paths
            .iter()
            .flatten()
            .find(|p| !same_path(p, &legacy_expected))
        {
            return Ok(RegistrationStatus::OtherCopy {
                manifest: other.clone(),
            });
        }
        if paths.iter().all(Option::is_none) {
            return Ok(if legacy_paths.iter().any(Option::is_some) {
                RegistrationStatus::NeedsRepair {
                    message:
                        "Reconnect this browser to update its Download Engine host registration"
                            .into(),
                }
            } else {
                RegistrationStatus::Missing
            });
        }
        let checked = (|| -> Result<String> {
            let manifest: HostManifest = serde_json::from_slice(&fs::read(&expected)?)?;
            ensure!(
                manifest.name == HOST_NAME
                    && manifest.transport == "stdio"
                    && same_path(&manifest.path, &self.root.join(HOST_PROGRAM))
                    && manifest.path.is_file(),
                "Host manifest or executable needs repair"
            );
            manifest.extension_id(browser, HOST_NAME, FIREFOX_ID)
        })();
        Ok(match checked {
            Ok(extension_id) => RegistrationStatus::Registered { extension_id },
            Err(error) => RegistrationStatus::NeedsRepair {
                message: error.to_string(),
            },
        })
    }
    pub fn register(&self, browser: Browser, id: &str) -> Result<()> {
        let id = browser.extension_id(id)?;
        ensure!(
            self.root.is_absolute(),
            "The application folder must be absolute"
        );
        ensure!(
            self.root.join(HOST_PROGRAM).is_file() && self.root.join("de.exe").is_file(),
            "Keep de.exe and de-host.exe together. Use the packaged application"
        );
        let _lock = self.lock()?;
        self.register_locked(browser, &id)
    }
    // Both identities belong to one connection. Commit or restore both manifests
    // and both registry views while holding the shared per-user lock.
    fn register_locked(&self, browser: Browser, id: &str) -> Result<()> {
        let targets = [
            (
                self.manifest_path(browser),
                self.registry_key(browser),
                HOST_NAME,
                FIREFOX_ID,
            ),
            (
                self.legacy_manifest_path(browser),
                self.legacy_registry_key(browser),
                LEGACY_HOST_NAME,
                LEGACY_FIREFOX_ID,
            ),
        ];
        let mut changes = Vec::new();
        for (path, key, name, firefox_id) in targets {
            let previous_paths = self.paths_at(&key)?;
            ensure!(
                previous_paths.iter().flatten().all(|p| same_path(p, &path)),
                "Another Download Engine copy is registered for {}. Disconnect it in that copy's Browser setup first",
                browser.label()
            );
            // Fresh installs authorize only the new identity. Retain the old
            // Firefox ID only where this copy already had a legacy connection.
            if name == LEGACY_HOST_NAME && previous_paths.iter().all(Option::is_none) {
                continue;
            }
            let previous_manifest = match fs::read(&path) {
                Ok(data) => Some(data),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    return Err(error).context("Cannot preserve the browser host manifest");
                }
            };
            changes.push(RegistrationChange {
                path,
                key,
                previous_paths,
                previous_manifest,
                manifest: HostManifest {
                    name: name.into(),
                    description: HOST_DESCRIPTION.into(),
                    path: self.root.join(HOST_PROGRAM),
                    transport: "stdio".into(),
                    allowed_origins: if browser == Browser::Firefox {
                        vec![]
                    } else {
                        vec![format!("chrome-extension://{id}/")]
                    },
                    allowed_extensions: if browser == Browser::Firefox {
                        vec![firefox_id.into()]
                    } else {
                        vec![]
                    },
                },
            });
        }
        let mut written = 0;
        let result = (|| -> Result<()> {
            for change in &changes {
                self.write_manifest(&change.path, &change.manifest)?;
                written += 1;
                for view in registry::VIEWS {
                    registry::write(&change.key, view, &change.path)?;
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            let mut rollback_errors = Vec::new();
            for change in changes[..written].iter().rev() {
                let restored = match &change.previous_manifest {
                    Some(data) => self.write_manifest_bytes(&change.path, data),
                    None => fs::remove_file(&change.path).map_err(anyhow::Error::from),
                };
                if let Err(rollback) = restored {
                    rollback_errors.push(rollback.to_string());
                }
                for (&view, previous) in registry::VIEWS.iter().zip(&change.previous_paths) {
                    let restored = match previous {
                        Some(path) => registry::write(&change.key, view, path),
                        None => registry::delete(&change.key, view),
                    };
                    if let Err(rollback) = restored {
                        rollback_errors.push(rollback.to_string());
                    }
                }
            }
            if !rollback_errors.is_empty() {
                return Err(error).context(format!(
                    "Registration rollback also failed: {}",
                    rollback_errors.join("; ")
                ));
            }
            return Err(error).context("Cannot register the browser host");
        }
        Ok(())
    }
    /// Removes both identities only where they point to this copy. Used on uninstall.
    pub fn unregister(&self, browser: Browser) -> Result<bool> {
        let _lock = self.lock()?;
        let mut removed = false;
        for (key, expected) in [
            (self.registry_key(browser), self.manifest_path(browser)),
            (
                self.legacy_registry_key(browser),
                self.legacy_manifest_path(browser),
            ),
        ] {
            for (&view, path) in registry::VIEWS.iter().zip(self.paths_at(&key)?) {
                if path.as_ref().is_some_and(|p| same_path(p, &expected)) {
                    registry::delete(&key, view)?;
                    removed = true;
                }
            }
        }
        Ok(removed)
    }
    /// Upgrades this copy's connections to the branded host identity and executable.
    /// The old identity remains a separate alias for extensions not yet updated.
    /// Other copies and unrecognized manifests are never adopted or overwritten.
    pub fn migrate_legacy_host(&self) -> Result<Vec<Browser>> {
        let host = self.root.join(HOST_PROGRAM);
        let manifests = Browser::ALL.iter().any(|&browser| {
            self.manifest_path(browser).is_file() || self.legacy_manifest_path(browser).is_file()
        });
        if !host.is_file() || !self.root.join("de.exe").is_file() || !manifests {
            return Ok(Vec::new());
        }
        let _lock = self.lock()?;
        let mut migrated = Vec::new();
        for browser in Browser::ALL {
            let current_path = self.manifest_path(browser);
            let legacy_path = self.legacy_manifest_path(browser);
            let current_paths = self.registered_paths(browser)?;
            let legacy_paths = self.paths_at(&self.legacy_registry_key(browser))?;
            if current_paths
                .iter()
                .flatten()
                .any(|p| !same_path(p, &current_path))
                || legacy_paths
                    .iter()
                    .flatten()
                    .any(|p| !same_path(p, &legacy_path))
            {
                continue;
            }
            let (path, name, firefox_id) = if current_paths.iter().any(Option::is_some) {
                (&current_path, HOST_NAME, FIREFOX_ID)
            } else if legacy_paths.iter().any(Option::is_some) {
                (&legacy_path, LEGACY_HOST_NAME, LEGACY_FIREFOX_ID)
            } else {
                continue;
            };
            let Ok(data) = fs::read(path) else {
                continue;
            };
            let Ok(manifest) = serde_json::from_slice::<HostManifest>(&data) else {
                continue;
            };
            let Ok(id) = manifest.extension_id(browser, name, firefox_id) else {
                continue;
            };
            if !same_path(&manifest.path, &host)
                && !same_path(&manifest.path, &self.root.join(LEGACY_HOST_PROGRAM))
            {
                continue;
            }
            let current = [
                (&current_path, &current_paths, HOST_NAME, FIREFOX_ID),
                (
                    &legacy_path,
                    &legacy_paths,
                    LEGACY_HOST_NAME,
                    LEGACY_FIREFOX_ID,
                ),
            ]
            .iter()
            .all(|(path, paths, name, firefox_id)| {
                if *name == LEGACY_HOST_NAME && paths.iter().all(Option::is_none) {
                    return true;
                }
                if paths.iter().any(Option::is_none) {
                    return false;
                }
                let Ok(data) = fs::read(path) else {
                    return false;
                };
                let Ok(manifest) = serde_json::from_slice::<HostManifest>(&data) else {
                    return false;
                };
                same_path(&manifest.path, &host)
                    && manifest
                        .extension_id(browser, name, firefox_id)
                        .is_ok_and(|value| {
                            value
                                == if browser == Browser::Firefox {
                                    *firefox_id
                                } else {
                                    &id
                                }
                        })
            });
            if current {
                continue;
            }
            self.register_locked(browser, &id)?;
            migrated.push(browser);
        }
        Ok(migrated)
    }
    fn write_manifest(&self, path: &Path, manifest: &HostManifest) -> Result<()> {
        self.write_manifest_bytes(path, &serde_json::to_vec_pretty(manifest)?)
    }
    fn write_manifest_bytes(&self, path: &Path, data: &[u8]) -> Result<()> {
        // Replacing the directory entry avoids truncating a preexisting linked file.
        let temporary = self
            .root
            .join(format!(".native-{}.tmp", uuid::Uuid::new_v4()));
        let write_result = (|| -> Result<()> {
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)?;
            file.write_all(data)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, path)?;
            Ok(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        write_result.context("Cannot save the browser host manifest")
    }
}
/// Chromium's ID for an extension loaded unpacked (Developer mode): SHA-256 of
/// the UTF-16 path with an upper-case drive letter, its first 16 bytes written
/// as hex digits mapped to the letters a–p.
pub fn unpacked_extension_id(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let text = path.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    let mut units: Vec<u16> = text.encode_utf16().collect();
    if units.len() >= 2 && units[1] == u16::from(b':') && (0x61..=0x7a).contains(&units[0]) {
        units[0] -= 0x20;
    }
    let bytes: Vec<u8> = units.iter().flat_map(|unit| unit.to_le_bytes()).collect();
    Sha256::digest(&bytes)[..16]
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .map(|digit| char::from(b'a' + digit))
        .collect()
}

fn same_path(left: &Path, right: &Path) -> bool {
    left.to_string_lossy()
        .replace('/', "\\")
        .eq_ignore_ascii_case(&right.to_string_lossy().replace('/', "\\"))
}

#[cfg(windows)]
mod registry {
    use super::*;
    use std::{
        ffi::OsString,
        os::windows::ffi::{OsStrExt, OsStringExt},
        ptr,
    };
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::*,
    };
    pub const VIEWS: [u32; 2] = [KEY_WOW64_32KEY, KEY_WOW64_64KEY];
    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }
    fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }
    fn check(code: u32) -> Result<()> {
        if code != ERROR_SUCCESS {
            return Err(std::io::Error::from_raw_os_error(code as i32).into());
        }
        Ok(())
    }
    pub fn read(path: &str, view: u32) -> Result<Option<PathBuf>> {
        let path = wide(path.as_ref());
        let mut handle = ptr::null_mut();
        // All pointers reference live UTF-16 strings or writable, correctly-sized buffers.
        let code = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                path.as_ptr(),
                0,
                KEY_QUERY_VALUE | view,
                &mut handle,
            )
        };
        if code == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(code)?;
        let key = Key(handle);
        let mut kind = 0;
        let mut length = 0;
        let code = unsafe {
            RegQueryValueExW(
                key.0,
                ptr::null(),
                ptr::null(),
                &mut kind,
                ptr::null_mut(),
                &mut length,
            )
        };
        if code == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(code)?;
        ensure!(
            kind == REG_SZ && (2..=65536).contains(&length) && length % 2 == 0,
            "Invalid native-host registry value"
        );
        let mut data = vec![0u16; length as usize / 2];
        check(unsafe {
            RegQueryValueExW(
                key.0,
                ptr::null(),
                ptr::null(),
                &mut kind,
                data.as_mut_ptr().cast(),
                &mut length,
            )
        })?;
        ensure!(
            kind == REG_SZ && length >= 2 && length % 2 == 0,
            "Invalid native-host registry value"
        );
        data.truncate(length as usize / 2);
        ensure!(
            data.pop() == Some(0) && !data.contains(&0),
            "Invalid native-host registry string"
        );
        let path = PathBuf::from(OsString::from_wide(&data));
        ensure!(
            path.is_absolute(),
            "Native-host registry path must be absolute"
        );
        Ok(Some(path))
    }
    pub fn write(path: &str, view: u32, value: &Path) -> Result<()> {
        let path = wide(path.as_ref());
        let value = wide(value.as_os_str());
        let mut handle = ptr::null_mut();
        check(unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                path.as_ptr(),
                0,
                ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE | view,
                ptr::null(),
                &mut handle,
                ptr::null_mut(),
            )
        })?;
        let key = Key(handle);
        check(unsafe {
            RegSetValueExW(
                key.0,
                ptr::null(),
                0,
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * 2) as u32,
            )
        })
    }
    pub fn delete(path: &str, view: u32) -> Result<()> {
        let path = wide(path.as_ref());
        let code = unsafe { RegDeleteKeyExW(HKEY_CURRENT_USER, path.as_ptr(), view, 0) };
        if code == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        check(code)
    }
    #[cfg(test)]
    pub fn cleanup(path: &str) {
        let path = wide(path.as_ref());
        // Only the caller's freshly generated test namespace is recursively removed.
        unsafe {
            RegDeleteTreeW(HKEY_CURRENT_USER, path.as_ptr());
        }
    }
}
#[cfg(not(windows))]
mod registry {
    use super::*;
    pub const VIEWS: [u32; 1] = [0];
    pub fn read(_: &str, _: u32) -> Result<Option<PathBuf>> {
        anyhow::bail!("Browser setup is supported on Windows")
    }
    pub fn write(_: &str, _: u32, _: &Path) -> Result<()> {
        anyhow::bail!("Browser setup is supported on Windows")
    }
    pub fn delete(_: &str, _: u32) -> Result<()> {
        anyhow::bail!("Browser setup is supported on Windows")
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    struct Fixture {
        setup: BrowserSetup,
        _temp: tempfile::TempDir,
    }
    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("Program folder");
            fs::create_dir(&root).unwrap();
            for name in ["de.exe", "de-host.exe"] {
                fs::write(root.join(name), b"fixture").unwrap();
            }
            Self {
                setup: BrowserSetup {
                    root,
                    lock_directory: temp.path().join("locks"),
                    registry_prefix: format!(r"Software\IDM_EX\Tests\{}", uuid::Uuid::new_v4()),
                },
                _temp: temp,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            registry::cleanup(&self.setup.registry_prefix);
        }
    }
    const ID: &str = "abcdefghijklmnopabcdefghijklmnop";
    fn seed_legacy(setup: &BrowserSetup, browser: Browser, program: &str) {
        let manifest = HostManifest {
            name: LEGACY_HOST_NAME.into(),
            description: "Previous release".into(),
            path: setup.root.join(program),
            transport: "stdio".into(),
            allowed_origins: if browser == Browser::Firefox {
                vec![]
            } else {
                vec![format!("chrome-extension://{ID}/")]
            },
            allowed_extensions: if browser == Browser::Firefox {
                vec![LEGACY_FIREFOX_ID.into()]
            } else {
                vec![]
            },
        };
        let path = setup.legacy_manifest_path(browser);
        fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        for view in registry::VIEWS {
            registry::write(&setup.legacy_registry_key(browser), view, &path).unwrap();
        }
    }

    #[test]
    fn old_connections_migrate_with_separate_exact_aliases_and_preserve_user_data() {
        let fixture = Fixture::new();
        let setup = &fixture.setup;
        let data = setup.root.join("download-state.keep");
        fs::write(&data, "saved download progress").unwrap();
        for browser in Browser::ALL {
            seed_legacy(
                setup,
                browser,
                if browser == Browser::Chrome {
                    LEGACY_HOST_PROGRAM
                } else {
                    HOST_PROGRAM
                },
            );
        }
        assert_eq!(setup.migrate_legacy_host().unwrap(), Browser::ALL);
        assert_eq!(fs::read_to_string(data).unwrap(), "saved download progress");
        for browser in Browser::ALL {
            for (name, firefox_id, path, key) in [
                (
                    HOST_NAME,
                    FIREFOX_ID,
                    setup.manifest_path(browser),
                    setup.registry_key(browser),
                ),
                (
                    LEGACY_HOST_NAME,
                    LEGACY_FIREFOX_ID,
                    setup.legacy_manifest_path(browser),
                    setup.legacy_registry_key(browser),
                ),
            ] {
                let manifest: HostManifest =
                    serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                assert_eq!(manifest.path, setup.root.join(HOST_PROGRAM));
                assert_eq!(
                    manifest.extension_id(browser, name, firefox_id).unwrap(),
                    if browser == Browser::Firefox {
                        firefox_id
                    } else {
                        ID
                    }
                );
                for view in registry::VIEWS {
                    assert_eq!(registry::read(&key, view).unwrap(), Some(path.clone()));
                }
            }
            assert_eq!(
                setup.status(browser).unwrap(),
                RegistrationStatus::Registered {
                    extension_id: if browser == Browser::Firefox {
                        FIREFOX_ID
                    } else {
                        ID
                    }
                    .into(),
                }
            );
            setup
                .register(
                    browser,
                    if browser == Browser::Firefox {
                        FIREFOX_ID
                    } else {
                        "pppppppppppppppppppppppppppppppp"
                    },
                )
                .unwrap();
            let legacy: HostManifest =
                serde_json::from_slice(&fs::read(setup.legacy_manifest_path(browser)).unwrap())
                    .unwrap();
            assert_eq!(
                legacy
                    .extension_id(browser, LEGACY_HOST_NAME, LEGACY_FIREFOX_ID)
                    .unwrap(),
                if browser == Browser::Firefox {
                    LEGACY_FIREFOX_ID
                } else {
                    "pppppppppppppppppppppppppppppppp"
                }
            );
        }
        assert!(setup.migrate_legacy_host().unwrap().is_empty());
    }

    #[test]
    fn identity_upgrade_never_adopts_untrusted_or_foreign_registrations() {
        for scenario in 0..7 {
            let fixture = Fixture::new();
            let setup = &fixture.setup;
            seed_legacy(setup, Browser::Chrome, HOST_PROGRAM);
            let path = setup.legacy_manifest_path(Browser::Chrome);
            let mut manifest: HostManifest =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            match scenario {
                0 => manifest.name = "com.someone.else".into(),
                1 => manifest.path = fixture._temp.path().join("other/de-host.exe"),
                2 => manifest.transport = "unknown".into(),
                3 => manifest
                    .allowed_origins
                    .push("chrome-extension://pppppppppppppppppppppppppppppppp/".into()),
                4 => manifest.allowed_extensions.push(LEGACY_FIREFOX_ID.into()),
                5 => {
                    registry::write(
                        &setup.legacy_registry_key(Browser::Chrome),
                        registry::VIEWS[1],
                        &fixture._temp.path().join("other/native-chrome.json"),
                    )
                    .unwrap();
                }
                6 => {
                    registry::write(
                        &setup.registry_key(Browser::Chrome),
                        registry::VIEWS[0],
                        &fixture
                            ._temp
                            .path()
                            .join("other/native-download-engine-chrome.json"),
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
            fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
            let before = fs::read(&path).unwrap();
            let old_registry = setup
                .paths_at(&setup.legacy_registry_key(Browser::Chrome))
                .unwrap();
            let new_registry = setup.registered_paths(Browser::Chrome).unwrap();
            assert!(
                setup.migrate_legacy_host().unwrap().is_empty(),
                "scenario {scenario}"
            );
            assert_eq!(fs::read(&path).unwrap(), before);
            assert!(!setup.manifest_path(Browser::Chrome).exists());
            assert_eq!(
                setup
                    .paths_at(&setup.legacy_registry_key(Browser::Chrome))
                    .unwrap(),
                old_registry
            );
            assert_eq!(
                setup.registered_paths(Browser::Chrome).unwrap(),
                new_registry
            );
            if scenario >= 5 {
                assert!(setup.register(Browser::Chrome, ID).is_err());
            }
        }
    }

    #[test]
    fn failed_alias_write_restores_both_files_and_registry_views() {
        use std::os::windows::fs::OpenOptionsExt;
        for existing in [false, true] {
            let fixture = Fixture::new();
            let setup = &fixture.setup;
            seed_legacy(setup, Browser::Chrome, HOST_PROGRAM);
            if existing {
                setup.register(Browser::Chrome, ID).unwrap();
            }
            let primary_before = fs::read(setup.manifest_path(Browser::Chrome)).ok();
            let legacy_before = fs::read(setup.legacy_manifest_path(Browser::Chrome)).unwrap();
            let primary_registry = setup.registered_paths(Browser::Chrome).unwrap();
            let legacy_registry = setup
                .paths_at(&setup.legacy_registry_key(Browser::Chrome))
                .unwrap();
            // Denying delete sharing exercises a real mid-transaction Windows failure.
            let _locked = fs::OpenOptions::new()
                .read(true)
                .share_mode(1)
                .open(setup.legacy_manifest_path(Browser::Chrome))
                .unwrap();
            assert!(
                setup
                    .register(Browser::Chrome, "pppppppppppppppppppppppppppppppp")
                    .is_err()
            );
            assert_eq!(
                fs::read(setup.manifest_path(Browser::Chrome)).ok(),
                primary_before
            );
            assert_eq!(
                fs::read(setup.legacy_manifest_path(Browser::Chrome)).unwrap(),
                legacy_before
            );
            assert_eq!(
                setup.registered_paths(Browser::Chrome).unwrap(),
                primary_registry
            );
            assert_eq!(
                setup
                    .paths_at(&setup.legacy_registry_key(Browser::Chrome))
                    .unwrap(),
                legacy_registry
            );
            assert!(!fs::read_dir(&setup.root).unwrap().any(|entry| {
                entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp")
            }));
        }
    }

    #[test]
    fn disconnect_removes_our_aliases_but_keeps_other_copies() {
        let fixture = Fixture::new();
        let setup = &fixture.setup;
        setup.register(Browser::Firefox, FIREFOX_ID).unwrap();
        let other = fixture._temp.path().join("other/native-firefox.json");
        registry::write(
            &setup.legacy_registry_key(Browser::Firefox),
            registry::VIEWS[1],
            &other,
        )
        .unwrap();
        // HKCU can expose the same key in both WOW64 views. Preserve the foreign
        // mappings Windows actually exposes, without assuming registry redirection.
        let foreign_paths = setup
            .paths_at(&setup.legacy_registry_key(Browser::Firefox))
            .unwrap();
        assert!(
            foreign_paths
                .iter()
                .flatten()
                .all(|path| same_path(path, &other))
        );
        assert!(setup.unregister(Browser::Firefox).unwrap());
        assert!(
            setup
                .registered_paths(Browser::Firefox)
                .unwrap()
                .iter()
                .all(Option::is_none)
        );
        assert_eq!(
            setup
                .paths_at(&setup.legacy_registry_key(Browser::Firefox))
                .unwrap(),
            foreign_paths
        );
    }
    #[test]
    fn registration_round_trip_is_scoped_and_exact_in_both_registry_views() {
        let fixture = Fixture::new();
        let setup = &fixture.setup;
        for browser in Browser::ALL {
            assert_eq!(setup.status(browser).unwrap(), RegistrationStatus::Missing);
            setup
                .register(browser, if browser == Browser::Firefox { "" } else { ID })
                .unwrap();
            assert!(
                setup
                    .paths_at(&setup.legacy_registry_key(browser))
                    .unwrap()
                    .iter()
                    .all(Option::is_none),
                "A fresh install must not authorize the old host or Firefox identity"
            );
            assert!(!setup.legacy_manifest_path(browser).exists());
            assert!(setup.migrate_legacy_host().unwrap().is_empty());
            let expected_id = if browser == Browser::Firefox {
                FIREFOX_ID
            } else {
                ID
            };
            assert_eq!(
                setup.status(browser).unwrap(),
                RegistrationStatus::Registered {
                    extension_id: expected_id.into()
                }
            );
            for view in registry::VIEWS {
                assert_eq!(
                    registry::read(&setup.registry_key(browser), view).unwrap(),
                    Some(setup.manifest_path(browser))
                );
            }
            let text = fs::read_to_string(setup.manifest_path(browser)).unwrap();
            assert!(!text.contains('*'));
            assert!(setup.unregister(browser).unwrap());
            assert!(
                setup
                    .paths_at(&setup.legacy_registry_key(browser))
                    .unwrap()
                    .iter()
                    .all(Option::is_none)
            );
            assert_eq!(setup.status(browser).unwrap(), RegistrationStatus::Missing);
            assert!(!setup.unregister(browser).unwrap());
        }
    }
    #[test]
    fn upgrade_points_this_copys_legacy_manifests_at_the_renamed_host() {
        let fixture = Fixture::new();
        let setup = &fixture.setup;
        let legacy = setup.root.join(LEGACY_HOST_PROGRAM);
        // As left by a pre-2.0 registration: same manifest and registry, old host name.
        for browser in [Browser::Chrome, Browser::Firefox] {
            setup
                .register(browser, if browser == Browser::Firefox { "" } else { ID })
                .unwrap();
            let path = setup.manifest_path(browser);
            let mut manifest: HostManifest =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            manifest.path = legacy.clone();
            fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
            assert!(matches!(
                setup.status(browser).unwrap(),
                RegistrationStatus::NeedsRepair { .. }
            ));
        }
        // Another copy's Edge registration names a legacy host of its own.
        let other = fixture._temp.path().join("other/native-edge.json");
        for view in registry::VIEWS {
            registry::write(&setup.registry_key(Browser::Edge), view, &other).unwrap();
        }

        assert_eq!(
            setup.migrate_legacy_host().unwrap(),
            [Browser::Chrome, Browser::Firefox]
        );
        assert_eq!(
            setup.status(Browser::Chrome).unwrap(),
            RegistrationStatus::Registered {
                extension_id: ID.into()
            }
        );
        assert_eq!(
            setup.status(Browser::Firefox).unwrap(),
            RegistrationStatus::Registered {
                extension_id: FIREFOX_ID.into()
            }
        );
        assert_eq!(
            setup.status(Browser::Edge).unwrap(),
            RegistrationStatus::OtherCopy { manifest: other }
        );
        assert!(!setup.manifest_path(Browser::Brave).exists());
        // Already migrated manifests are left as they are.
        assert!(setup.migrate_legacy_host().unwrap().is_empty());
    }
    #[test]
    fn invalid_extension_ids_never_create_registration_or_manifests() {
        let fixture = Fixture::new();
        for invalid in [
            "",
            "*",
            "chrome-extension://abcdefghijklmnopabcdefghijklmnop/",
            "ABCDEFGHIJKLMNOPABCDEFGHIJKLMNOP",
            "abcdefghijklmnopabcdefghijklmnox",
            "abcd\0ef",
        ] {
            assert!(fixture.setup.register(Browser::Chrome, invalid).is_err());
        }
        assert!(!fixture.setup.manifest_path(Browser::Chrome).exists());
        assert_eq!(
            fixture.setup.status(Browser::Chrome).unwrap(),
            RegistrationStatus::Missing
        );
    }
    #[test]
    fn another_copy_in_either_registry_view_is_preserved() {
        let fixture = Fixture::new();
        let other = fixture._temp.path().join("other/native-chrome.json");
        for view in registry::VIEWS {
            registry::write(&fixture.setup.registry_key(Browser::Chrome), view, &other).unwrap();
            assert!(fixture.setup.register(Browser::Chrome, ID).is_err());
            assert!(!fixture.setup.unregister(Browser::Chrome).unwrap());
            assert_eq!(
                fixture.setup.status(Browser::Chrome).unwrap(),
                RegistrationStatus::OtherCopy {
                    manifest: other.clone()
                }
            );
            assert_eq!(
                registry::read(&fixture.setup.registry_key(Browser::Chrome), view).unwrap(),
                Some(other.clone())
            );
            registry::delete(&fixture.setup.registry_key(Browser::Chrome), view).unwrap();
        }
    }
    #[test]
    fn registration_repairs_missing_manifest_and_changes_only_selected_browser() {
        let fixture = Fixture::new();
        fixture.setup.register(Browser::Chrome, ID).unwrap();
        fs::remove_file(fixture.setup.manifest_path(Browser::Chrome)).unwrap();
        assert!(matches!(
            fixture.setup.status(Browser::Chrome).unwrap(),
            RegistrationStatus::NeedsRepair { .. }
        ));
        let new_id = "pppppppppppppppppppppppppppppppp";
        fixture.setup.register(Browser::Chrome, new_id).unwrap();
        assert_eq!(
            fixture.setup.status(Browser::Chrome).unwrap(),
            RegistrationStatus::Registered {
                extension_id: new_id.into()
            }
        );
        assert_eq!(
            fixture.setup.status(Browser::Edge).unwrap(),
            RegistrationStatus::Missing
        );
    }
    #[test]
    fn unpacked_ids_match_what_chromium_browsers_show_for_these_folders() {
        // Chrome and Brave showed these IDs for the local test copy's folders.
        let root = r"C:\Users\Tommy\Desktop\IDM_EX\dist\IDM-EX-UnsignedDev\extensions";
        for (folder, id) in [
            ("chrome", "aldojdbffhahojhmbkbnjhfmbjoidfpg"),
            ("brave", "ejdcchcgambklhfegfjioigmbdomalgk"),
            ("edge", "becncclgfieoodeljhaplnppipdiogae"),
        ] {
            let path = PathBuf::from(format!(r"{root}\{folder}"));
            assert_eq!(unpacked_extension_id(&path), id, "{folder}");
            assert!(Browser::Chrome.extension_id(id).is_ok());
        }
        // Chromium upper-cases the drive letter and ignores the verbatim prefix.
        let chrome = format!(r"{root}\chrome");
        for variant in [format!("c{}", &chrome[1..]), format!(r"\\?\{chrome}")] {
            assert_eq!(
                unpacked_extension_id(Path::new(&variant)),
                "aldojdbffhahojhmbkbnjhfmbjoidfpg"
            );
        }
    }
    #[test]
    fn expected_ids_come_only_from_this_copys_packaged_extension_folders() {
        let fixture = Fixture::new();
        let setup = &fixture.setup;
        assert_eq!(setup.expected_extension_id(Browser::Brave), None);
        for browser in Browser::ALL {
            let folder = setup.extension_folder(browser);
            fs::create_dir_all(&folder).unwrap();
            fs::write(folder.join("manifest.json"), b"{}").unwrap();
        }
        let ids: Vec<_> = Browser::ALL
            .into_iter()
            .map(|browser| setup.expected_extension_id(browser))
            .collect();
        assert_eq!(ids[3], None, "Firefox keeps its built-in ID");
        for (browser, id) in Browser::ALL.into_iter().zip(&ids).take(3) {
            let id = id.as_deref().unwrap();
            assert_eq!(id, unpacked_extension_id(&setup.extension_folder(browser)));
            setup.register(browser, id).unwrap();
        }
        assert_ne!(ids[0], ids[2], "Chrome and Brave folders get different IDs");
    }
    #[test]
    fn brave_has_its_own_folder_and_registration_separate_from_chrome() {
        let fixture = Fixture::new();
        let setup = &fixture.setup;
        assert_eq!(Browser::parse("brave").unwrap(), Browser::Brave);
        assert_eq!(
            setup.extension_folder(Browser::Brave),
            setup.root.join(r"extensions\brave")
        );
        let brave_id = "pppppppppppppppppppppppppppppppp";
        setup.register(Browser::Chrome, ID).unwrap();
        setup.register(Browser::Brave, brave_id).unwrap();
        // Brave checks this key before Chrome's; it has no key of its own on Windows.
        let chromium_key = format!(
            r"{}\Chromium\NativeMessagingHosts\{HOST_NAME}",
            setup.registry_prefix
        );
        for view in registry::VIEWS {
            assert_eq!(
                registry::read(&chromium_key, view).unwrap(),
                Some(setup.manifest_path(Browser::Brave))
            );
        }
        let manifest = fs::read_to_string(setup.manifest_path(Browser::Brave)).unwrap();
        assert!(manifest.contains(&format!("chrome-extension://{brave_id}/")));
        assert!(!manifest.contains(ID));
        assert!(setup.unregister(Browser::Brave).unwrap());
        assert_eq!(
            setup.status(Browser::Brave).unwrap(),
            RegistrationStatus::Missing
        );
        assert_eq!(
            setup.status(Browser::Chrome).unwrap(),
            RegistrationStatus::Registered {
                extension_id: ID.into()
            }
        );
    }
    #[test]
    fn registration_cannot_truncate_a_linked_manifest_target() {
        let fixture = Fixture::new();
        let victim = fixture._temp.path().join("keep.txt");
        fs::write(&victim, "keep this content").unwrap();
        fs::hard_link(&victim, fixture.setup.manifest_path(Browser::Chrome)).unwrap();
        fixture.setup.register(Browser::Chrome, ID).unwrap();
        assert_eq!(fs::read_to_string(victim).unwrap(), "keep this content");
    }
    #[test]
    fn missing_host_fails_without_changing_registry() {
        let fixture = Fixture::new();
        fs::remove_file(fixture.setup.root.join("de-host.exe")).unwrap();
        assert!(fixture.setup.register(Browser::Chrome, ID).is_err());
        assert_eq!(
            fixture.setup.status(Browser::Chrome).unwrap(),
            RegistrationStatus::Missing
        );
    }
    #[test]
    fn concurrent_registration_fails_cleanly_without_writing() {
        let fixture = Fixture::new();
        let _guard = fixture.setup.lock().unwrap();
        assert!(
            fixture
                .setup
                .register(Browser::Firefox, "")
                .unwrap_err()
                .to_string()
                .contains("already running")
        );
        assert!(!fixture.setup.manifest_path(Browser::Firefox).exists());
    }
}
