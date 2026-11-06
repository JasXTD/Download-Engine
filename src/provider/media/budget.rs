//! Bound expansion across all tracks, including checkpoints from earlier runs.
use anyhow::{Context, Result, ensure};
use std::{
    fs::File,
    io::{self, Write},
    path::Path,
};

const OUTPUT_OVERHEAD: u64 = 16 * 1024 * 1024;
const FREE_HEADROOM: u64 = 64 * 1024 * 1024;

pub(crate) struct OutputBudget {
    limit: u64,
    used: u64,
}
impl OutputBudget {
    pub(crate) fn new(input: u64) -> Result<Self> {
        let limit = input
            .checked_mul(2)
            .and_then(|n| n.checked_add(OUTPUT_OVERHEAD))
            .context("Media output budget overflow")?;
        Ok(Self { limit, used: 0 })
    }
    pub(crate) fn include(&mut self, size: u64) -> Result<()> {
        let used = self
            .used
            .checked_add(size)
            .context("Media output size overflow")?;
        ensure!(
            used <= self.limit,
            "Media output exceeds the job-wide expansion limit (twice the input size plus 16 MiB)"
        );
        self.used = used;
        Ok(())
    }
    pub(crate) fn allowance(&self, segment_limit: u64) -> Result<u64> {
        let remaining = self
            .limit
            .checked_sub(self.used)
            .context("Media output budget exhausted")?;
        ensure!(remaining > 0, "Media output budget exhausted");
        Ok(remaining.min(segment_limit))
    }
    pub(crate) fn used(&self) -> u64 {
        self.used
    }
}

pub(crate) fn mux_limit(size: u64) -> Result<u64> {
    size.checked_add(size / 2)
        .and_then(|n| n.checked_add(8 * 1024 * 1024))
        .context("Media mux size overflow")
}

pub(crate) fn available(folder: &Path) -> Result<u64> {
    #[cfg(windows)]
    {
        crate::torrent::free_space(folder).context("Cannot check free space for media processing")
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let folder = folder
            .ancestors()
            .find(|p| p.is_dir())
            .context("Missing media storage directory")?;
        let name = std::ffi::CString::new(folder.as_os_str().as_bytes())?;
        let mut info = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        // SAFETY: name is NUL-terminated and info is valid writable storage.
        if unsafe { libc::statvfs(name.as_ptr(), info.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error())
                .context("Cannot check free space for media processing");
        }
        // SAFETY: statvfs succeeded and initialized info. f_bavail honors reserved blocks.
        let info = unsafe { info.assume_init() };
        u64::try_from((info.f_bavail as u128) * (info.f_frsize as u128))
            .context("Available media space overflow")
    }
}
pub(crate) fn check_space(folder: &Path, bytes: u64) -> Result<()> {
    let required = bytes
        .checked_add(FREE_HEADROOM)
        .context("Media space requirement overflow")?;
    ensure!(
        available(folder)? >= required,
        "Not enough free space for media processing; keep 64 MiB free beyond temporary and output files"
    );
    Ok(())
}

/// Account conservatively as if cache, temp and destination share one volume.
/// Recheck before each segment: this is a preflight, not an OS space reservation.
pub(crate) fn check_processing_space(
    cache: &Path,
    destination: &Path,
    saved: u64,
    next: u64,
    spool_input: Option<u64>,
) -> Result<()> {
    let total = saved
        .checked_add(next)
        .context("Media output size overflow")?;
    let mux = mux_limit(total)?;
    let scratch = match spool_input {
        Some(input) => input
            .checked_add(next)
            .context("Media scratch size overflow")?,
        None => 0,
    };
    let cache_bytes = next
        .checked_add(total)
        .and_then(|n| n.checked_add(mux))
        .and_then(|n| n.checked_add(scratch))
        .context("Media space requirement overflow")?;
    check_space(cache, cache_bytes)?;
    check_space(destination, mux)?;
    if scratch > 0 {
        check_space(&std::env::temp_dir(), scratch)?;
    }
    Ok(())
}

/// Check again at each bounded write in case another job consumed free space.
pub(crate) struct DiskWriter<'a> {
    pub(crate) file: &'a mut File,
    pub(crate) directory: &'a Path,
}
impl Write for DiskWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        check_space(self.directory, bytes.len() as u64).map_err(io::Error::other)?;
        self.file.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_output_consumes_the_same_allowance_as_new_tracks() {
        let mut budget = OutputBudget::new(10).unwrap();
        budget.include(OUTPUT_OVERHEAD + 12).unwrap();
        assert_eq!(budget.allowance(512 << 20).unwrap(), 8);
        assert!(budget.include(9).is_err());
        assert_eq!(budget.allowance(512 << 20).unwrap(), 8);
        budget.include(8).unwrap();
        assert!(budget.allowance(1).is_err());
        assert!(OutputBudget::new(u64::MAX).is_err());
        assert!(mux_limit(u64::MAX).is_err());
    }
    #[test]
    fn disk_checks_fail_closed_without_allocating_large_files() {
        let root = tempfile::tempdir().unwrap();
        assert!(available(root.path()).unwrap() > 0);
        check_space(root.path(), 1).unwrap();
        // Deterministic even if another process frees space between queries.
        assert!(check_space(root.path(), u64::MAX - FREE_HEADROOM).is_err());
        assert!(check_space(root.path(), u64::MAX).is_err());
        assert!(check_processing_space(root.path(), root.path(), u64::MAX, 1, None).is_err());
    }
}
