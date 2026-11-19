//! Windows package identity and stable entry points. Ordinary installations and
//! portable copies keep their existing paths; no folder-name heuristic is used.
use anyhow::Result;

pub const HOST_ALIAS: &str = "download-engine-host.exe";
pub const STARTUP_TASK: &str = "DownloadEngineStartup";
pub fn open_store_updates() -> Result<()> {
    use anyhow::Context;
    open_system_page("ms-windows-store://downloadsandupdates")
        .context("Cannot open Microsoft Store")
}

/// Both callers supply a fixed Windows protocol URI. ShellExecute returns a
/// status, not an owned handle; the terminated URI stays live throughout the call.
#[cfg(windows)]
pub(crate) fn open_system_page(uri: &'static str) -> Result<()> {
    let uri: Vec<u16> = uri.encode_utf16().chain(Some(0)).collect();
    let result = unsafe {
        windows_sys::Win32::UI::Shell::ShellExecuteW(
            std::ptr::null_mut(),
            std::ptr::null(),
            uri.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        )
    } as isize;
    anyhow::ensure!(
        result > 32,
        "Windows could not open the page (code {result})"
    );
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn open_system_page(_: &'static str) -> Result<()> {
    anyhow::bail!("Windows integration is only available on Windows")
}

#[cfg(windows)]
pub fn family_name() -> Result<Option<String>> {
    use windows_sys::Win32::{
        Foundation::{APPMODEL_ERROR_NO_PACKAGE, ERROR_SUCCESS},
        Storage::Packaging::Appx::GetCurrentPackageFamilyName,
    };
    // PACKAGE_FAMILY_NAME_MAX_LENGTH is 65. Keep room for future OS extensions;
    // the API receives the actual capacity and cannot overrun this buffer.
    let mut buffer = [0u16; 256];
    let mut length = buffer.len() as u32;
    let code = unsafe { GetCurrentPackageFamilyName(&mut length, buffer.as_mut_ptr()) };
    if code == APPMODEL_ERROR_NO_PACKAGE {
        return Ok(None);
    }
    anyhow::ensure!(
        code == ERROR_SUCCESS,
        "Cannot read Windows package identity (code {code})"
    );
    anyhow::ensure!(
        length > 1 && length as usize <= buffer.len(),
        "Invalid Windows package identity"
    );
    let family = String::from_utf16(&buffer[..length as usize - 1])?;
    anyhow::ensure!(
        family
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b)),
        "Invalid Windows package family name"
    );
    Ok(Some(family))
}

#[cfg(not(windows))]
pub fn family_name() -> Result<Option<String>> {
    Ok(None)
}

pub fn is_packaged() -> bool {
    // Fail closed for update/registration UI: an unexpected identity API error
    // must not offer the unpackaged installer or rewrite external associations.
    !matches!(family_name(), Ok(None))
}

#[cfg(windows)]
pub fn browser_locations(
    program_root: &std::path::Path,
    local_app_data: &std::path::Path,
    family: Option<&str>,
) -> (std::path::PathBuf, std::path::PathBuf) {
    match family {
        Some(family) => (
            local_app_data
                .join("Download Engine")
                .join("Browser")
                .join(family),
            local_app_data
                .join("Microsoft/WindowsApps")
                .join(family)
                .join(HOST_ALIAS),
        ),
        None => (program_root.to_owned(), program_root.join("de-host.exe")),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn unpackaged_test_binary_has_no_package_identity() {
        assert_eq!(super::family_name().unwrap(), None);
        assert!(!super::is_packaged());
    }

    #[cfg(windows)]
    #[test]
    fn browser_paths_are_stable_across_package_updates_and_separate_from_portable() {
        use super::*;
        use std::path::Path;
        let local = Path::new(r"C:\Users\Example\AppData\Local");
        let old = Path::new(r"C:\Program Files\WindowsApps\Example_1.0.0.0_x64");
        let new = Path::new(r"C:\Program Files\WindowsApps\Example_1.0.1.0_x64");
        let packaged = browser_locations(old, local, Some("Example_publisher"));
        assert_eq!(
            packaged,
            browser_locations(new, local, Some("Example_publisher"))
        );
        assert!(!packaged.0.starts_with(old));
        assert!(!packaged.1.starts_with(old));
        assert_eq!(
            browser_locations(old, local, None),
            (old.to_owned(), old.join("de-host.exe"))
        );
        assert_ne!(
            packaged,
            browser_locations(new, local, Some("Other_publisher"))
        );
    }
}
