//! Unix saved-file operations. Anchor names to directory descriptors, refuse
//! replacement, and verify an opened file's identity before removing it.
use anyhow::{Context, Result, ensure};
use std::{
    ffi::{CStr, CString},
    fs::{File, OpenOptions},
    io,
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawFd, FromRawFd, RawFd},
    },
    path::{Path, PathBuf},
};

fn name(value: &std::ffi::OsStr) -> io::Result<CString> {
    CString::new(value.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid file name"))
}

fn open_at(directory: RawFd, name: &CStr) -> io::Result<File> {
    // NONBLOCK prevents an unexpected FIFO from hanging the desktop.
    let fd = unsafe {
        libc::openat(
            directory,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // openat returned a new owned descriptor.
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn parent(path: &Path) -> io::Result<(File, CString, PathBuf)> {
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Expected an absolute path",
        ));
    }
    let leaf = name(
        path.file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Missing file name"))?,
    )?;
    // Resolve a selected folder once (including macOS /var -> /private/var).
    // All subsequent operations use opened directory descriptors, not that path.
    let resolved = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Missing parent folder"))?
        .canonicalize()?;
    let mut directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open("/")?;
    for part in resolved.components() {
        if let std::path::Component::Normal(part) = part {
            let next = open_at(directory.as_raw_fd(), &name(part)?)?;
            if !next.metadata()?.is_dir() {
                return Err(io::Error::other("Parent is not a directory"));
            }
            directory = next;
        }
    }
    Ok((directory, leaf, resolved))
}

fn same_file(first: &File, second: &File) -> io::Result<bool> {
    let first = first.metadata()?;
    let second = second.metadata()?;
    Ok(first.dev() == second.dev() && first.ino() == second.ino())
}

fn rename_at(from_dir: RawFd, from: &CStr, to_dir: RawFd, to: &CStr) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::renameat2(
            from_dir,
            from.as_ptr(),
            to_dir,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    #[cfg(target_os = "macos")]
    let result = unsafe {
        libc::renameatx_np(
            from_dir,
            from.as_ptr(),
            to_dir,
            to.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    return Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Atomic no-replace rename is unavailable",
    ));
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub(crate) fn rename_owned(file: &File, from: &Path, to: &Path) -> Result<()> {
    let (source, source_name, _) = parent(from)?;
    let (target, target_name, _) = parent(to)?;
    ensure!(
        same_file(file, &open_at(source.as_raw_fd(), &source_name)?)?,
        "Source changed; move preserved all files"
    );
    rename_at(
        source.as_raw_fd(),
        &source_name,
        target.as_raw_fd(),
        &target_name,
    )
    .context("Cannot move the file without replacing another file")?;
    if !open_at(target.as_raw_fd(), &target_name)
        .and_then(|actual| same_file(file, &actual))
        .is_ok_and(|same| same)
    {
        rename_at(
            target.as_raw_fd(),
            &target_name,
            source.as_raw_fd(),
            &source_name,
        )
        .context("File changed during the move; destination retained for review")?;
        anyhow::bail!("File changed during the move; original name restored");
    }
    Ok(())
}

pub(crate) fn delete_owned(file: &File, path: &Path) -> Result<()> {
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() || metadata.is_dir(),
        "Expected a regular file or folder"
    );
    let (parent, leaf, parent_path) = parent(path)?;
    ensure!(
        same_file(file, &open_at(parent.as_raw_fd(), &leaf)?)?,
        "File changed; deletion preserved it"
    );
    // Claim the name in a private sibling directory before unlinking it. This
    // prevents other users from swapping it after the identity check. A failed
    // removal is restored without overwriting anything; no recursive deletion.
    let staging_name = CString::new(format!(".download-engine-remove-{}", uuid::Uuid::new_v4()))?;
    let staging_path = parent_path.join(staging_name.to_str()?);
    if unsafe { libc::mkdirat(parent.as_raw_fd(), staging_name.as_ptr(), 0o700) } != 0 {
        return Err(io::Error::last_os_error()).context("Cannot prepare saved-file removal");
    }
    let result = (|| -> Result<()> {
        let staging = open_at(parent.as_raw_fd(), &staging_name)?;
        rename_at(parent.as_raw_fd(), &leaf, staging.as_raw_fd(), c"entry")?;
        let removed = (|| -> Result<()> {
            ensure!(
                same_file(file, &open_at(staging.as_raw_fd(), c"entry")?)?,
                "File changed during removal"
            );
            let flag = if metadata.is_dir() {
                libc::AT_REMOVEDIR
            } else {
                0
            };
            if unsafe { libc::unlinkat(staging.as_raw_fd(), c"entry".as_ptr(), flag) } != 0 {
                return Err(io::Error::last_os_error().into());
            }
            Ok(())
        })();
        if let Err(error) = removed {
            rename_at(staging.as_raw_fd(), c"entry", parent.as_raw_fd(), &leaf).with_context(
                || {
                    format!(
                        "Removal failed; file retained at {}",
                        staging_path.join("entry").display()
                    )
                },
            )?;
            return Err(error).context("Removal failed; original name restored");
        }
        Ok(())
    })();
    // Only an empty staging directory can be removed here. Recovery files stay.
    let cleaned = unsafe {
        libc::unlinkat(
            parent.as_raw_fd(),
            staging_name.as_ptr(),
            libc::AT_REMOVEDIR,
        )
    };
    if result.is_ok() && cleaned != 0 {
        return Err(io::Error::last_os_error())
            .context("File removed, but its empty staging folder needs cleanup");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_and_colliding_destination_are_preserved() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let target = root.path().join("target");
        std::fs::write(&source, b"original").unwrap();
        std::fs::write(&target, b"keep").unwrap();
        let opened = File::open(&source).unwrap();
        assert!(rename_owned(&opened, &source, &target).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
        std::fs::rename(&source, root.path().join("saved-original")).unwrap();
        std::fs::write(&source, b"replacement").unwrap();
        assert!(delete_owned(&opened, &source).is_err());
        assert_eq!(std::fs::read(&source).unwrap(), b"replacement");
        assert_eq!(
            std::fs::read(root.path().join("saved-original")).unwrap(),
            b"original"
        );
    }
    #[test]
    fn deletes_files_and_empty_folders_but_preserves_nonempty_folders_and_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("folder");
        std::fs::create_dir(&folder).unwrap();
        let path = folder.join("file");
        std::fs::write(&path, b"safe").unwrap();
        let directory = File::open(&folder).unwrap();
        assert!(delete_owned(&directory, &folder).is_err());
        assert!(path.is_file());
        let file = File::open(&path).unwrap();
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(delete_owned(&file, &link).is_err());
        delete_owned(&file, &path).unwrap();
        delete_owned(&directory, &folder).unwrap();
        assert!(!folder.exists());
        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }
    #[test]
    fn directory_rename_uses_no_replace_semantics() {
        let root = tempfile::tempdir().unwrap();
        let from = root.path().join("before");
        let to = root.path().join("after");
        std::fs::create_dir(&from).unwrap();
        std::fs::write(from.join("entry"), b"payload").unwrap();
        let file = File::open(&from).unwrap();
        rename_owned(&file, &from, &to).unwrap();
        assert!(!from.exists());
        assert_eq!(std::fs::read(to.join("entry")).unwrap(), b"payload");
    }
}
