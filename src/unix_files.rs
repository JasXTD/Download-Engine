//! Unix saved-file operations. Anchor names to directory descriptors, refuse
//! replacement, and verify an opened file's identity before removing it.
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
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

/// Claim a hard link in an owner-only directory before exposing the final name.
/// The original link remains the crash-recovery anchor. Both link and rename use
/// the private directory descriptor even if a shared parent is rearranged.
pub(crate) struct VerifiedLink {
    directory: de_torrent::storage::Directory,
    path: PathBuf,
    linked: bool,
}

impl VerifiedLink {
    pub(crate) fn new(file: &File, source: &Path, path: PathBuf) -> Result<Self> {
        let source = name(source.as_os_str())?;
        let directory = de_torrent::storage::Directory::create_new(&path)?;
        let mut link = Self {
            directory,
            path,
            linked: false,
        };
        let fd = link.directory.handle().as_raw_fd();
        if unsafe { libc::linkat(libc::AT_FDCWD, source.as_ptr(), fd, c"entry".as_ptr(), 0) } != 0 {
            return Err(io::Error::last_os_error())
                .context("Cannot claim the download for publication");
        }
        link.linked = true;
        ensure!(
            same_file(file, &open_at(fd, c"entry")?)?,
            "Verified download was replaced before publication"
        );
        Ok(link)
    }

    pub(crate) fn publish(&mut self, destination: &File, leaf: &std::ffi::OsStr) -> io::Result<()> {
        rename_at(
            self.directory.handle().as_raw_fd(),
            c"entry",
            destination.as_raw_fd(),
            &name(leaf)?,
        )?;
        self.linked = false;
        Ok(())
    }

    /// A crash before the final rename can leave an extra link on a resumable
    /// part. Remove it only when the private entry still matches a known anchor.
    pub(crate) fn recover(path: &Path, anchors: impl Iterator<Item = PathBuf>) -> Result<()> {
        let directory = match de_torrent::storage::Directory::open(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            result => result?,
        };
        de_torrent::storage::require_private_directory(directory.handle())?;
        match open_at(directory.handle().as_raw_fd(), c"entry") {
            Ok(entry) => {
                let mut matches = false;
                for anchor in anchors {
                    match open_at(libc::AT_FDCWD, &name(anchor.as_os_str())?) {
                        Ok(file) if file.metadata()?.is_file() && same_file(&file, &entry)? => {
                            matches = true;
                            break;
                        }
                        Ok(_) => {}
                        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                        Err(e) => return Err(e.into()),
                    }
                }
                ensure!(
                    matches,
                    "Unrecognized file in the unfinished publication folder"
                );
                directory.remove(Path::new("entry"), false)?;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        delete_owned(directory.handle(), path)
    }
}

impl Drop for VerifiedLink {
    fn drop(&mut self) {
        if self.linked {
            let _ = self.directory.remove(Path::new("entry"), false);
        }
        let _ = delete_owned(self.directory.handle(), &self.path);
    }
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

// The private intermediate name works for directories as well as files. Its
// name is stable so the existing move journals can recover a process exit
// between the two renames. The lock excludes recovery by a second app process.
struct MoveClaim {
    parent: File,
    leaf: CString,
    directory: File,
    name: CString,
    path: PathBuf,
}

impl MoveClaim {
    fn open(from: &Path, create: bool) -> Result<Option<Self>> {
        let (parent, leaf, parent_path) = parent(from)?;
        let digest = Sha256::digest(leaf.to_bytes());
        let folder = format!(".download-engine-move-{digest:x}");
        let name = CString::new(folder.as_str())?;
        let fd = parent.as_raw_fd();
        if create && unsafe { libc::mkdirat(fd, name.as_ptr(), 0o700) } != 0 {
            let err = io::Error::last_os_error();
            if err.kind() != io::ErrorKind::AlreadyExists {
                return Err(err).context("Cannot create the private move folder");
            }
        }
        let directory = match open_at(fd, &name) {
            Err(e) if !create && e.kind() == io::ErrorKind::NotFound => return Ok(None),
            result => result?,
        };
        de_torrent::storage::require_private_directory(&directory)?;
        if unsafe { libc::flock(directory.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::last_os_error()).context("Another move is using this folder");
        }
        ensure!(
            same_file(&directory, &open_at(fd, &name)?)?,
            "The move folder changed"
        );
        Ok(Some(Self {
            parent,
            leaf,
            directory,
            name,
            path: parent_path.join(folder),
        }))
    }

    fn restore(&self) -> Result<()> {
        rename_at(
            self.directory.as_raw_fd(),
            c"entry",
            self.parent.as_raw_fd(),
            &self.leaf,
        )
        .map_err(|e| {
            // A collision here is a recovery error, not a collision at the publication target.
            anyhow::anyhow!(
                "Cannot restore the original name ({e}); entry preserved at {}",
                self.path.join("entry").display()
            )
        })
    }

    fn recover(&self, expected: impl FnOnce(&File) -> Result<bool>) -> Result<()> {
        let entry = match open_at(self.directory.as_raw_fd(), c"entry") {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            result => result?,
        };
        ensure!(
            expected(&entry)?,
            "Unrecognized move entry preserved at {}",
            self.path.join("entry").display()
        );
        self.restore()
    }

    fn take(&self) -> Result<()> {
        rename_at(
            self.parent.as_raw_fd(),
            &self.leaf,
            self.directory.as_raw_fd(),
            c"entry",
        )?;
        Ok(())
    }

    fn publish(&self, file: &File, target: &File, target_name: &CStr) -> Result<()> {
        let fd = self.directory.as_raw_fd();
        let result = (|| -> Result<()> {
            ensure!(
                same_file(file, &open_at(fd, c"entry")?)?,
                "Source changed during the move"
            );
            rename_at(fd, c"entry", target.as_raw_fd(), target_name)
                .context("Cannot move the file without replacing another file")
        })();
        if result.is_err() {
            self.restore()?;
        }
        result
    }
}

impl Drop for MoveClaim {
    fn drop(&mut self) {
        let fd = self.parent.as_raw_fd();
        if open_at(fd, &self.name)
            .and_then(|dir| same_file(&self.directory, &dir))
            .unwrap_or(false)
        {
            // rmdir leaves any retained entry intact. Once publication succeeds,
            // an empty staging directory is never grounds for retrying the move.
            unsafe {
                libc::unlinkat(fd, self.name.as_ptr(), libc::AT_REMOVEDIR);
            }
        }
    }
}

pub(crate) fn restore_interrupted_move(
    from: &Path,
    expected: impl FnOnce(&File) -> Result<bool>,
) -> Result<()> {
    if let Some(claim) = MoveClaim::open(from, false)? {
        claim.recover(expected)?;
    }
    Ok(())
}

pub(crate) fn rename_owned(file: &File, from: &Path, to: &Path) -> Result<()> {
    let (target, target_name, _) = parent(to)?;
    let claim = MoveClaim::open(from, true)?.context("Missing private move folder")?;
    claim.recover(|entry| Ok(same_file(file, entry)?))?;
    ensure!(
        same_file(file, &open_at(claim.parent.as_raw_fd(), &claim.leaf)?)?,
        "Source changed before the move"
    );
    claim.take()?;
    claim.publish(file, &target, &target_name)
}

pub(crate) fn delete_owned(file: &File, path: &Path) -> Result<()> {
    let (parent, leaf, parent_path) = parent(path)?;
    delete_in(file, &parent, &leaf, &parent_path)
}

pub(crate) fn delete_relative(file: &File, directory: &File, leaf: &std::ffi::OsStr) -> Result<()> {
    delete_in(file, directory, &name(leaf)?, Path::new("."))
}

fn delete_in(file: &File, parent: &File, leaf: &CStr, parent_path: &Path) -> Result<()> {
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() || metadata.is_dir(),
        "Expected a regular file or folder"
    );
    ensure!(
        same_file(file, &open_at(parent.as_raw_fd(), leaf)?)?,
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
    let staging = open_at(parent.as_raw_fd(), &staging_name)?;
    de_torrent::storage::require_private_directory(&staging)?;
    let result = (|| -> Result<()> {
        rename_at(parent.as_raw_fd(), leaf, staging.as_raw_fd(), c"entry")?;
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
            rename_at(staging.as_raw_fd(), c"entry", parent.as_raw_fd(), leaf).with_context(
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

    #[cfg(target_os = "macos")]
    #[test]
    fn extended_acl_cannot_grant_access_to_a_private_move_directory() {
        use std::os::unix::fs::DirBuilderExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("private");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        let directory = File::open(&path).unwrap();
        de_torrent::storage::require_private_directory(&directory).unwrap();
        assert!(
            std::process::Command::new("/bin/chmod")
                .args(["+a", "everyone allow add_file"])
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        assert_eq!(
            de_torrent::storage::require_private_directory(&directory)
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
    }

    #[test]
    fn an_unverified_entry_stays_private_even_when_the_source_name_is_occupied() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let other = temp.path().join("other");
        std::fs::write(&source, b"first").unwrap();
        std::fs::write(&other, b"second").unwrap();
        let claim = MoveClaim::open(&source, true).unwrap().unwrap();
        claim.take().unwrap();
        std::fs::write(&source, b"third").unwrap();
        let target = File::open(temp.path()).unwrap();
        let error = claim
            .publish(&File::open(other).unwrap(), &target, c"saved")
            .unwrap_err();
        assert!(error.to_string().contains("entry preserved"));
        assert!(!temp.path().join("saved").exists());
        assert_eq!(std::fs::read(&source).unwrap(), b"third");
        assert_eq!(std::fs::read(claim.path.join("entry")).unwrap(), b"first");
        drop(claim);
        assert!(restore_interrupted_move(&source, |_| Ok(true)).is_err());
    }

    #[test]
    fn interrupted_move_recovers_the_original_directory_before_retry() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let saved = temp.path().join("saved");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("item"), b"keep").unwrap();
        let file = File::open(&source).unwrap();
        let expected = |entry: &File| Ok(same_file(&file, entry)?);
        let claim = MoveClaim::open(&source, true).unwrap().unwrap();
        claim.take().unwrap();
        assert!(restore_interrupted_move(&source, expected).is_err());
        drop(claim);
        assert!(restore_interrupted_move(&source, |_| Ok(false)).is_err());
        assert!(!source.exists());
        restore_interrupted_move(&source, expected).unwrap();
        restore_interrupted_move(&source, expected).unwrap();
        rename_owned(&file, &source, &saved).unwrap();
        assert_eq!(std::fs::read(saved.join("item")).unwrap(), b"keep");
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    }

    #[test]
    fn publication_uses_the_verified_file_across_name_changes_and_collisions() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("partial");
        let saved = root.path().join("saved");
        std::fs::write(&source, b"verified bytes").unwrap();
        let file = File::open(&source).unwrap();
        let mut link = VerifiedLink::new(&file, &source, root.path().join("claim")).unwrap();
        std::fs::rename(&source, root.path().join("original")).unwrap();
        std::fs::write(&source, b"other bytes").unwrap();
        assert!(VerifiedLink::new(&file, &source, root.path().join("other-claim")).is_err());

        let directory = File::open(root.path()).unwrap();
        std::fs::write(&saved, b"existing file").unwrap();
        assert_eq!(
            link.publish(&directory, "saved".as_ref())
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        link.publish(&directory, "available".as_ref()).unwrap();
        drop(link);
        assert_eq!(std::fs::read(saved).unwrap(), b"existing file");
        assert_eq!(
            std::fs::read(root.path().join("available")).unwrap(),
            b"verified bytes"
        );
        assert_eq!(std::fs::read(source).unwrap(), b"other bytes");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 4);
    }

    #[test]
    fn recovery_releases_an_interrupted_link_only_for_its_own_anchor() {
        let temp = tempfile::tempdir().unwrap();
        let part = temp.path().join("part");
        let claim = temp.path().join("claim");
        std::fs::write(&part, b"original").unwrap();
        drop(de_torrent::storage::Directory::create_new(&claim).unwrap());
        std::fs::hard_link(&part, claim.join("entry")).unwrap();
        assert!(VerifiedLink::recover(&claim, std::iter::empty()).is_err());
        assert_eq!(std::fs::read(claim.join("entry")).unwrap(), b"original");
        VerifiedLink::recover(&claim, std::iter::once(part.clone())).unwrap();
        assert!(!claim.exists());
        assert_eq!(std::fs::metadata(&part).unwrap().nlink(), 1);
        assert_eq!(std::fs::read(part).unwrap(), b"original");
    }
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
