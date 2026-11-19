use anyhow::{Context, Result};
use std::{io, path::Path};

/// Space available to this user on the volume that will hold the requested folder.
pub fn available_space(folder: &Path) -> Result<u64> {
    let folder = folder
        .ancestors()
        .find(|p| p.is_dir())
        .context("Missing storage directory")?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let name: Vec<_> = folder.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut free = 0;
        // SAFETY: name is NUL-terminated and free is a valid output for this user.
        if unsafe {
            GetDiskFreeSpaceExW(
                name.as_ptr(),
                &mut free,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error().into());
        }
        Ok(free)
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let name = std::ffi::CString::new(folder.as_os_str().as_bytes())?;
        let mut info = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        // SAFETY: name is NUL-terminated and info is writable.
        if unsafe { libc::statvfs(name.as_ptr(), info.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        // SAFETY: statvfs initialized info. f_bavail excludes reserved blocks.
        let info = unsafe { info.assume_init() };
        u64::try_from((info.f_bavail as u128) * (info.f_frsize as u128))
            .context("Available space overflow")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn free_space_uses_the_existing_parent_without_creating_the_destination() {
        let temp = crate::testkit::TempFolder::new();
        let missing = temp.path().join("new").join("download");
        assert!(super::available_space(&temp.path()).unwrap() > 0);
        assert!(super::available_space(&missing).unwrap() > 0);
        assert!(!missing.exists());
    }
}
