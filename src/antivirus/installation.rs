//! Read-only discovery of installed scan integrations. Finding a program is not
//! evidence that its engine, signatures, or license are ready to scan.
use super::{Provider, Settings};
use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};

pub fn check_setup(settings: &Settings) -> Result<PathBuf> {
    settings.validate()?;
    let path = match settings.provider {
        Provider::Defender => super::defender_path()?,
        provider if provider.external_review() => handler(provider)?,
        _ => PathBuf::from(&settings.executable),
    };
    let _parents = super::process::guard_parents(&path)?;
    let _file = crate::download_file::read_completed(&path)?;
    Ok(path)
}

pub fn open_antivirus(provider: Provider) -> Result<()> {
    ensure!(
        provider.external_review(),
        "Open this antivirus from the Start menu"
    );
    let dll = handler(provider)?;
    let folder = dll.parent().context("Missing antivirus folder")?;
    let names: &[&str] = match provider {
        Provider::Malwarebytes => &["Malwarebytes.exe"],
        Provider::Kaspersky => &["avpui.exe"],
        Provider::Bitdefender => &["seccenter.exe"],
        _ => unreachable!(),
    };
    let app = names
        .iter()
        .map(|name| folder.join(name))
        .find(|path| path.is_file())
        .context("The antivirus app shortcut is unavailable. Open it from the Start menu.")?;
    let _parents = super::process::guard_parents(&app)?;
    let _file = crate::download_file::read_completed(&app)?;
    crate::paths::open_download(&app)
}

pub(super) fn handler(provider: Provider) -> Result<PathBuf> {
    ensure!(
        provider.external_review(),
        "This scanner does not use Explorer"
    );
    #[cfg(windows)]
    {
        windows::handler(provider)
    }
    #[cfg(not(windows))]
    {
        anyhow::bail!("Explorer antivirus integrations require Windows")
    }
}

fn vendor_path(provider: Provider, path: &Path, roots: &[PathBuf]) -> bool {
    let vendor = match provider {
        Provider::Malwarebytes => "malwarebytes",
        Provider::Kaspersky => "kaspersky lab",
        Provider::Bitdefender => "bitdefender",
        _ => return false,
    };
    // Compare path components, not string prefixes (Bitdefender-Other is not Bitdefender).
    let components: Vec<_> = path.components().collect();
    if components
        .iter()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return false;
    }
    roots.iter().any(|root| {
        let root: Vec<_> = root.components().collect();
        components.len() > root.len() + 1
            && components
                .iter()
                .zip(&root)
                .all(|(a, b)| a.as_os_str().eq_ignore_ascii_case(b.as_os_str()))
            && components[root.len()]
                .as_os_str()
                .eq_ignore_ascii_case(vendor)
    }) && path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("dll"))
}

#[cfg(windows)]
fn class_id(name: &str, value: &str) -> Option<String> {
    [value, name].into_iter().find_map(|candidate| {
        let id = candidate
            .strip_prefix('{')
            .and_then(|s| s.strip_suffix('}'))
            .unwrap_or(candidate);
        windows_core::GUID::try_from(id).ok()?;
        Some(format!("{{{id}}}"))
    })
}

#[cfg(windows)]
mod windows {
    use super::*;
    use windows_sys::Win32::System::Registry::*;
    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    fn read(path: &str) -> Result<String> {
        let mut buffer = [0u16; 4096];
        let mut size = std::mem::size_of_val(&buffer) as u32;
        let result = unsafe {
            RegGetValueW(
                HKEY_CLASSES_ROOT,
                wide(path).as_ptr(),
                std::ptr::null(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        };
        ensure!(
            result == 0,
            "Explorer registration is unavailable ({result})"
        );
        let end = buffer
            .iter()
            .position(|c| *c == 0)
            .context("Invalid Explorer registration")?;
        Ok(String::from_utf16(&buffer[..end])?)
    }
    fn children(path: &str) -> Result<Vec<String>> {
        let mut key = std::ptr::null_mut();
        let result = unsafe {
            RegOpenKeyExW(
                HKEY_CLASSES_ROOT,
                wide(path).as_ptr(),
                0,
                KEY_READ,
                &mut key,
            )
        };
        if result == 2 {
            return Ok(Vec::new());
        }
        ensure!(result == 0, "Cannot read Explorer integrations ({result})");
        let key = Key(key);
        let mut keys = Vec::new();
        for index in 0..512 {
            let mut name = [0u16; 256];
            let mut size = name.len() as u32;
            let result = unsafe {
                RegEnumKeyExW(
                    key.0,
                    index,
                    name.as_mut_ptr(),
                    &mut size,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if result == 259 {
                return Ok(keys);
            }
            ensure!(
                result == 0,
                "Cannot enumerate Explorer integrations ({result})"
            );
            keys.push(String::from_utf16(&name[..size as usize])?);
        }
        anyhow::bail!("Too many Explorer integrations")
    }
    pub(super) fn handler(provider: Provider) -> Result<PathBuf> {
        use ::windows::Win32::UI::Shell::{FOLDERID_ProgramFiles, FOLDERID_ProgramFilesX86};
        let roots = [
            super::super::process::known_folder(&FOLDERID_ProgramFiles)?,
            super::super::process::known_folder(&FOLDERID_ProgramFilesX86)?,
        ];
        let mut found = Vec::new();
        for base in [
            r"*\shellex\ContextMenuHandlers",
            r"AllFilesystemObjects\shellex\ContextMenuHandlers",
        ] {
            for name in children(base)? {
                let registration = format!("{base}\\{name}");
                let value = read(&registration).unwrap_or_default();
                // Only a class identifier is accepted, never an arbitrary registry path.
                let Some(clsid) = class_id(&name, &value) else {
                    continue;
                };
                let Ok(value) = read(&format!("CLSID\\{clsid}\\InprocServer32")) else {
                    continue;
                };
                let path = PathBuf::from(value.trim_matches('"'));
                if vendor_path(provider, &path, &roots)
                    && path.is_file()
                    && !found
                        .iter()
                        .any(|p: &PathBuf| p.as_os_str().eq_ignore_ascii_case(path.as_os_str()))
                {
                    found.push(path);
                }
            }
        }
        ensure!(
            found.len() == 1,
            "{} Explorer scan integration is missing or ambiguous. Enable its Explorer integration in the antivirus app; custom installation locations are unsupported.",
            provider.name()
        );
        let path = found.remove(0);
        let _parents = super::super::process::guard_parents(&path)?;
        let _file = crate::download_file::read_completed(&path)?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn explorer_class_ids_accept_windows_braces_and_named_handler_keys() {
        let id = "{57CE581A-0CB6-4266-9CA0-19364C90A0B3}";
        assert_eq!(class_id("MBAMShlExt", id).as_deref(), Some(id));
        assert_eq!(class_id(id, "Friendly description").as_deref(), Some(id));
        assert_eq!(class_id(id, "").as_deref(), Some(id));
        assert_eq!(class_id("handler", &id[1..37]).as_deref(), Some(id));
        for invalid in [
            "",
            "CLSID\\Other",
            "{not-a-guid}",
            "{57CE581A-0CB6-4266-9CA0-19364C90A0B3}\\Other",
            "{57CE581A-0CB6-4266-9CA0-19364C90A0B3",
        ] {
            assert!(class_id("handler", invalid).is_none());
        }
    }
    #[test]
    fn setup_checks_only_read_configuration_and_files() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("clamscan.exe");
        let settings = Settings {
            provider: Provider::ClamAv,
            executable: path.display().to_string(),
            ..Settings::default()
        };
        assert!(check_setup(&settings).is_err());
        std::fs::write(&path, b"Harmless non-executable setup-check fixture").unwrap();
        assert_eq!(check_setup(&settings).unwrap(), path);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"Harmless non-executable setup-check fixture"
        );
        assert!(open_antivirus(Provider::Custom).is_err());
        temp.close().unwrap();
    }

    #[test]
    #[ignore = "Read-only host discovery diagnostic; never launches an antivirus"]
    fn installed_scanner_diagnostic() {
        for provider in [
            Provider::Defender,
            Provider::Malwarebytes,
            Provider::Kaspersky,
            Provider::Bitdefender,
        ] {
            println!(
                "{}: {:?}",
                provider.name(),
                check_setup(&Settings {
                    provider,
                    ..Settings::default()
                })
            );
        }
    }
    #[test]
    fn provider_discovery_rejects_other_vendors_prefixes_and_traversal() {
        let roots = [
            PathBuf::from(r"C:\Program Files"),
            PathBuf::from(r"C:\Program Files (x86)"),
        ];
        for (provider, vendor) in [
            (Provider::Malwarebytes, "Malwarebytes"),
            (Provider::Kaspersky, "Kaspersky Lab"),
            (Provider::Bitdefender, "Bitdefender"),
        ] {
            assert!(vendor_path(
                provider,
                &PathBuf::from(format!(r"c:\program files\{vendor}\Product\shell.DLL")),
                &roots
            ));
            for bad in [
                format!(r"C:\Downloads\{vendor}\shell.dll"),
                format!(r"C:\Program Files\{vendor}-Other\shell.dll"),
                format!(r"C:\Program Files\{vendor}\..\Other\shell.dll"),
                format!(r"C:\Program Files\{vendor}\shell.exe"),
            ] {
                assert!(!vendor_path(provider, Path::new(&bad), &roots), "{bad}");
            }
        }
        assert!(!vendor_path(
            Provider::Kaspersky,
            Path::new(r"C:\Program Files\Bitdefender\shell.dll"),
            &roots
        ));
    }
}
