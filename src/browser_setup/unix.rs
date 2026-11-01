//! Browser-owned per-user manifest locations on macOS and Linux; no registry.
use super::*;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

fn executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

pub struct BrowserSetup {
    root: PathBuf,
    resources: PathBuf,
    home: PathBuf,
    config: PathBuf,
    lock_directory: PathBuf,
}

impl BrowserSetup {
    pub fn discover() -> Result<Self> {
        let root = std::env::current_exe()?
            .parent()
            .context("Missing application folder")?
            .to_owned();
        let home = crate::paths::user_home()?;
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| home.join(".config"));
        let resources = crate::paths::resource_directory()?;
        Ok(Self {
            root,
            resources,
            home,
            config,
            lock_directory: crate::paths::user_data_root()?.join("Download Engine"),
        })
    }

    pub fn extension_folder(&self, browser: Browser) -> PathBuf {
        self.resources.join("extensions").join(browser.slug())
    }
    pub fn expected_extension_id(&self, browser: Browser) -> Option<String> {
        let folder = self.extension_folder(browser);
        (browser != Browser::Firefox && folder.join("manifest.json").is_file())
            .then(|| unpacked_extension_id(&folder))
    }
    fn manifest_path(&self, browser: Browser) -> PathBuf {
        manifest_directory(&self.home, &self.config, browser, cfg!(target_os = "macos"))
            .join(format!("{HOST_NAME}.json"))
    }
    fn host(&self) -> PathBuf {
        self.root.join("de-host")
    }
    fn lock(&self) -> Result<fs::File> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.lock_directory)?;
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(self.lock_directory.join("browser-registration.lock"))?;
        lock.try_lock()
            .context("Browser setup is already running. Try again when it finishes")?;
        Ok(lock)
    }
    fn read(&self, browser: Browser) -> Result<Option<HostManifest>> {
        let path = self.manifest_path(browser);
        let mut file = match fs::OpenOptions::new()
            .read(true)
            // Opening a FIFO must not block Browser setup before metadata can
            // reject it. Regular files are unaffected by O_NONBLOCK.
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("Cannot read the browser host manifest"),
        };
        ensure!(
            file.metadata()?.is_file() && file.metadata()?.len() <= 64 * 1024,
            "Invalid browser host manifest"
        );
        use std::io::Read;
        let mut bytes = Vec::new();
        (&mut file).take(64 * 1024 + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= 64 * 1024,
            "Browser host manifest is too large"
        );
        Ok(Some(
            serde_json::from_slice(&bytes).context("Invalid browser host manifest")?,
        ))
    }
    pub fn status(&self, browser: Browser) -> Result<RegistrationStatus> {
        let manifest = match self.read(browser) {
            Ok(Some(manifest)) => manifest,
            Ok(None) => return Ok(RegistrationStatus::Missing),
            Err(error) => {
                return Ok(RegistrationStatus::NeedsRepair {
                    message: error.to_string(),
                });
            }
        };
        if manifest.path != self.host() {
            return Ok(RegistrationStatus::OtherCopy {
                manifest: self.manifest_path(browser),
            });
        }
        match manifest.extension_id(browser, HOST_NAME, FIREFOX_ID) {
            Ok(extension_id) if executable(&self.host()) => {
                Ok(RegistrationStatus::Registered { extension_id })
            }
            Ok(_) => Ok(RegistrationStatus::NeedsRepair {
                message: "The native host is missing or not executable. Restore the complete application package."
                    .into(),
            }),
            Err(error) => Ok(RegistrationStatus::NeedsRepair {
                message: error.to_string(),
            }),
        }
    }
    pub fn register(&self, browser: Browser, id: &str) -> Result<()> {
        let extension_id = browser.extension_id(id)?;
        ensure!(
            self.root.is_absolute()
                && executable(&self.host())
                && executable(&self.root.join("de")),
            "Keep executable de and de-host files together. Use the complete packaged application"
        );
        let _lock = self.lock()?;
        // A different copy must explicitly disconnect first; never silently take it over.
        if let Some(previous) = self.read(browser)? {
            ensure!(
                previous.name == HOST_NAME && previous.path == self.host(),
                "Another Download Engine copy is registered. Disconnect it in that copy's Browser setup first"
            );
        }
        let path = self.manifest_path(browser);
        fs::create_dir_all(path.parent().context("Missing browser manifest folder")?)?;
        let manifest = HostManifest {
            name: HOST_NAME.into(),
            description: HOST_DESCRIPTION.into(),
            path: self.host(),
            transport: "stdio".into(),
            allowed_origins: if browser == Browser::Firefox {
                vec![]
            } else {
                vec![format!("chrome-extension://{extension_id}/")]
            },
            allowed_extensions: if browser == Browser::Firefox {
                vec![extension_id]
            } else {
                vec![]
            },
        };
        let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| -> Result<()> {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
            file.sync_all()?;
            fs::rename(&temporary, &path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.context("Cannot save the browser host manifest")
    }
    pub fn unregister(&self, browser: Browser) -> Result<bool> {
        let _lock = self.lock()?;
        let Some(manifest) = self.read(browser)? else {
            return Ok(false);
        };
        if manifest.name != HOST_NAME || manifest.path != self.host() {
            return Ok(false);
        }
        fs::remove_file(self.manifest_path(browser))?;
        Ok(true)
    }
    pub fn migrate_legacy_host(&self) -> Result<Vec<Browser>> {
        Ok(Vec::new())
    }
}

fn manifest_directory(home: &Path, config: &Path, browser: Browser, macos: bool) -> PathBuf {
    if macos {
        home.join("Library/Application Support")
            .join(match browser {
                Browser::Chrome => "Google/Chrome/NativeMessagingHosts",
                Browser::Edge => "Microsoft Edge/NativeMessagingHosts",
                Browser::Brave => "BraveSoftware/Brave-Browser/NativeMessagingHosts",
                Browser::Firefox => "Mozilla/NativeMessagingHosts",
            })
    } else if browser == Browser::Firefox {
        home.join(".mozilla/native-messaging-hosts")
    } else {
        config
            .join(match browser {
                Browser::Chrome => "google-chrome",
                Browser::Edge => "microsoft-edge",
                Browser::Brave => "BraveSoftware/Brave-Browser",
                Browser::Firefox => unreachable!(),
            })
            .join("NativeMessagingHosts")
    }
}

/// Chromium hashes FilePath's native bytes: UTF-8 on Unix, UTF-16 on Windows.
pub fn unpacked_extension_id(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(path.as_os_str().as_encoded_bytes())[..16]
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 15])
        .map(|digit| char::from(b'a' + digit))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, BrowserSetup) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Program 下載");
        fs::create_dir(&root).unwrap();
        for name in ["de", "de-host"] {
            fs::write(root.join(name), b"fixture").unwrap();
            fs::set_permissions(root.join(name), fs::Permissions::from_mode(0o700)).unwrap();
        }
        let setup = BrowserSetup {
            root: root.clone(),
            resources: root,
            home: temp.path().join("home"),
            config: temp.path().join("config"),
            lock_directory: temp.path().join("locks"),
        };
        (temp, setup)
    }
    #[test]
    fn native_registration_round_trip_and_other_copy_protection() {
        let (_temp, mut setup) = fixture();
        for browser in Browser::ALL {
            assert_eq!(setup.status(browser).unwrap(), RegistrationStatus::Missing);
            let id = if browser == Browser::Firefox {
                FIREFOX_ID
            } else {
                "abcdefghijklmnopabcdefghijklmnop"
            };
            setup.register(browser, id).unwrap();
            assert_eq!(
                setup.status(browser).unwrap(),
                RegistrationStatus::Registered {
                    extension_id: id.into()
                }
            );
            let original = fs::read(setup.manifest_path(browser)).unwrap();
            let root = setup.root.clone();
            setup.root = root.parent().unwrap().join("another copy");
            fs::create_dir_all(&setup.root).unwrap();
            for name in ["de", "de-host"] {
                fs::write(setup.root.join(name), b"fixture").unwrap();
            }
            assert!(matches!(
                setup.status(browser).unwrap(),
                RegistrationStatus::OtherCopy { .. }
            ));
            assert!(setup.register(browser, id).is_err());
            assert!(!setup.unregister(browser).unwrap());
            assert_eq!(fs::read(setup.manifest_path(browser)).unwrap(), original);
            setup.root = root;
            assert!(setup.unregister(browser).unwrap());
            assert_eq!(setup.status(browser).unwrap(), RegistrationStatus::Missing);
        }
    }
    #[test]
    fn platform_manifest_locations_follow_browser_contracts() {
        let home = Path::new("/home/test");
        let config = Path::new("/custom/config");
        assert_eq!(
            manifest_directory(home, config, Browser::Firefox, false),
            home.join(".mozilla/native-messaging-hosts")
        );
        assert_eq!(
            manifest_directory(home, config, Browser::Chrome, false),
            config.join("google-chrome/NativeMessagingHosts")
        );
        assert_eq!(
            manifest_directory(home, config, Browser::Edge, true),
            home.join("Library/Application Support/Microsoft Edge/NativeMessagingHosts")
        );
        assert_eq!(
            manifest_directory(home, config, Browser::Firefox, true),
            home.join("Library/Application Support/Mozilla/NativeMessagingHosts")
        );
    }
    #[test]
    fn registration_does_not_follow_a_manifest_symlink() {
        let (_temp, setup) = fixture();
        let path = setup.manifest_path(Browser::Firefox);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let outside = setup.root.join("keep.json");
        fs::write(&outside, b"keep").unwrap();
        std::os::unix::fs::symlink(&outside, path).unwrap();
        assert!(setup.register(Browser::Firefox, FIREFOX_ID).is_err());
        assert_eq!(fs::read(outside).unwrap(), b"keep");
    }

    #[test]
    fn a_fifo_manifest_is_rejected_without_blocking_browser_setup() {
        use std::os::unix::ffi::OsStrExt;
        let (_temp, setup) = fixture();
        let path = setup.manifest_path(Browser::Firefox);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || tx.send(setup.status(Browser::Firefox)).unwrap());
        let result = rx.recv_timeout(std::time::Duration::from_secs(2));
        if result.is_err() {
            // Release a regressed blocking reader before failing, including one
            // that has not yet reached open(). Never leave a hung test thread.
            let _reader = fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&path)
                .unwrap();
            let _writer = fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&path)
                .unwrap();
            worker.join().unwrap();
            panic!("Browser setup blocked on a FIFO manifest");
        }
        worker.join().unwrap();
        assert!(matches!(
            result.unwrap().unwrap(),
            RegistrationStatus::NeedsRepair { .. }
        ));
    }

    #[test]
    fn a_non_executable_host_is_not_reported_as_connected() {
        let (_temp, setup) = fixture();
        setup.register(Browser::Firefox, FIREFOX_ID).unwrap();
        fs::set_permissions(setup.host(), fs::Permissions::from_mode(0o600)).unwrap();
        assert!(matches!(
            setup.status(Browser::Firefox).unwrap(),
            RegistrationStatus::NeedsRepair { .. }
        ));
        assert!(setup.register(Browser::Firefox, FIREFOX_ID).is_err());
    }
}
