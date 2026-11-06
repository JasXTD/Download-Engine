//! An application-directory writer must not replace the executable after its
//! checksum is checked. Copy into an owned private directory, verify that copy,
//! then execute relative to its held directory descriptor. Renaming any ancestor
//! of either the resource or temporary directory cannot redirect the launch.
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{fs::MetadataExt, process::CommandExt},
    },
    path::PathBuf,
    process::Command,
    sync::Arc,
};
use tokio_util::sync::CancellationToken;

const MAX_PROGRAM_BYTES: u64 = 512 * 1024 * 1024;

pub(super) struct Program {
    directory: File,
    // A private directory limits access to this user. It is not a sandbox
    // against another process with complete control of the same user account.
    folder: PathBuf,
}

impl Drop for Program {
    fn drop(&mut self) {
        // Never recursively clean a pathname that an ancestor rename could
        // redirect. Remove only our file through the owned directory and then
        // the same empty directory, preserving replacements at its old name.
        unsafe {
            libc::unlinkat(self.directory.as_raw_fd(), c"ffmpeg".as_ptr(), 0);
        }
        let _ = crate::unix_files::delete_owned(&self.directory, &self.folder);
    }
}

impl Program {
    pub(super) fn verified(
        mut source: File,
        expected: &str,
        cancel: &CancellationToken,
    ) -> Result<Arc<Self>> {
        ensure!(expected.len() == 64, "Missing media merger checksum");
        ensure!(!cancel.is_cancelled(), "HLS merge cancelled");
        let size = source.metadata()?.len();
        ensure!(
            source.metadata()?.is_file() && size <= MAX_PROGRAM_BYTES,
            "The bundled media merger is not a bounded regular file"
        );
        let folder = tempfile::Builder::new()
            .prefix("download-engine-merger-")
            .tempdir()
            .context("Cannot create a private media merger directory")?;
        let directory = File::open(folder.path())?;
        let metadata = directory.metadata()?;
        ensure!(
            metadata.is_dir()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0,
            "The merger staging directory is not private"
        );
        let program = Self {
            directory,
            folder: folder.keep(),
        };
        // Creation and execution both use this descriptor, not the temporary
        // directory's pathname. Only this process receives a writable handle.
        let fd = unsafe {
            libc::openat(
                program.directory.as_raw_fd(),
                c"ffmpeg".as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                // macOS mode_t is u16; C variadic arguments require promotion.
                0o500 as libc::c_uint,
            )
        };
        ensure!(
            fd >= 0,
            "Cannot create the private media merger: {}",
            std::io::Error::last_os_error()
        );
        // SAFETY: openat returned a new descriptor; this File is its sole owner.
        let mut copy = unsafe { File::from_raw_fd(fd) };
        let mut hash = Sha256::new();
        let mut buffer = vec![0; 256 * 1024];
        let mut copied = 0u64;
        loop {
            ensure!(!cancel.is_cancelled(), "HLS merge cancelled");
            let count = source.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            copied += count as u64;
            ensure!(
                copied <= size,
                "The bundled media merger changed while being copied"
            );
            copy.write_all(&buffer[..count])?;
            hash.update(&buffer[..count]);
        }
        ensure!(
            copied == size && format!("{:x}", hash.finalize()) == expected,
            "The bundled media merger changed. Repair or re-extract Download Engine before downloading separate tracks."
        );
        ensure!(!cancel.is_cancelled(), "HLS merge cancelled");
        // A writable executable descriptor would cause ETXTBSY at exec on Linux.
        drop(copy);
        Ok(Arc::new(program))
    }

    pub(super) fn command(self: &Arc<Self>) -> Command {
        let program = self.clone();
        let mut command = Command::new("./ffmpeg");
        // All media inputs are absolute paths and stdout is an inherited file.
        // fchdir pins the private namespace even if a parent name is replaced.
        // Keeping the Arc in Command also covers delayed spawn/helper disposal.
        unsafe {
            command.pre_exec(move || {
                // SAFETY: the captured owner keeps this directory descriptor
                // live. fchdir and errno access are async-signal-safe after fork.
                if libc::fchdir(program.directory.as_raw_fd()) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        command
    }
}

#[cfg(test)]
#[path = "unix_program/tests.rs"]
mod tests;
