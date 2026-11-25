use super::directory::Creation;
use std::{
    ffi::{CString, OsStr},
    fs::{File, OpenOptions},
    io,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt},
        },
    },
    path::Path,
};

fn name(value: &OsStr) -> io::Result<CString> {
    CString::new(value.as_bytes()).map_err(|_| io::ErrorKind::InvalidInput.into())
}

fn opened(fd: libc::c_int) -> io::Result<File> {
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        // A successful openat returned a new owned descriptor.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

const DIRECTORY_FLAGS: libc::c_int =
    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;

pub(super) fn open_directory(path: &Path, _for_move: bool) -> io::Result<File> {
    // The user's selected path can have symlinked ancestors (e.g. /var on macOS).
    // Strip trailing slashes and "." so O_NOFOLLOW applies to the named directory.
    // Normalizing components does not resolve symlinks.
    let path: std::path::PathBuf = path.components().collect();
    OpenOptions::new()
        .read(true)
        .custom_flags(DIRECTORY_FLAGS)
        .open(path)
}

pub(super) fn directory_at(parent: &File, part: &OsStr, creation: Creation) -> io::Result<File> {
    let part = name(part)?;
    if creation != Creation::Open
        && unsafe { libc::mkdirat(parent.as_raw_fd(), part.as_ptr(), 0o700) } < 0
    {
        let error = io::Error::last_os_error();
        if creation == Creation::New || error.kind() != io::ErrorKind::AlreadyExists {
            return Err(error);
        }
    }
    let directory =
        opened(unsafe { libc::openat(parent.as_raw_fd(), part.as_ptr(), DIRECTORY_FLAGS) })?;
    if creation == Creation::New {
        require_private_directory(&directory)?;
    }
    Ok(directory)
}

/// Check the open directory itself before using it to protect a file operation.
pub fn require_private_directory(directory: &File) -> io::Result<()> {
    let metadata = directory.metadata()?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "The temporary folder is no longer private",
        ));
    }
    // On Darwin, inherited ACL grants can give access even with mode 0700.
    #[cfg(target_os = "macos")]
    verify_private_acl(directory.as_raw_fd())?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn verify_private_acl(fd: libc::c_int) -> io::Result<()> {
    use libc::{c_int, c_void};

    unsafe extern "C" {
        fn acl_get_fd(fd: c_int) -> *mut c_void;
        fn acl_free(acl: *mut c_void) -> c_int;
        fn acl_get_entry(acl: *mut c_void, selector: c_int, entry: *mut *mut c_void) -> c_int;
        fn acl_get_tag_type(entry: *mut c_void, tag: *mut c_int) -> c_int;
    }
    const ACL_FIRST_ENTRY: c_int = 0;
    const ACL_NEXT_ENTRY: c_int = -1;
    const ACL_EXTENDED_DENY: c_int = 2;

    let acl = unsafe { acl_get_fd(fd) };
    if acl.is_null() {
        let error = io::Error::last_os_error();
        return if error.raw_os_error() == Some(libc::ENOENT) {
            Ok(())
        } else {
            Err(error)
        };
    }
    let mut selector = ACL_FIRST_ENTRY;
    let result = loop {
        let mut entry = std::ptr::null_mut();
        if unsafe { acl_get_entry(acl, selector, &mut entry) } != 0 {
            // Darwin reports EINVAL at the end of a valid ACL.
            let error = io::Error::last_os_error();
            break if error.raw_os_error() == Some(libc::EINVAL) {
                Ok(())
            } else {
                Err(error)
            };
        }
        let mut tag = 0;
        if unsafe { acl_get_tag_type(entry, &mut tag) } != 0 {
            break Err(io::Error::last_os_error());
        }
        if tag != ACL_EXTENDED_DENY {
            break Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "The temporary folder has an ACL granting access",
            ));
        }
        selector = ACL_NEXT_ENTRY;
    };
    unsafe { acl_free(acl) };
    result
}

pub(super) fn identity(file: &File) -> io::Result<String> {
    let info = file.metadata()?;
    let birth = info
        .created()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok());
    Ok(format!(
        "{:x}:{:x}:{:x}",
        info.dev(),
        info.ino(),
        birth.map_or(0, |duration| duration.as_nanos())
    ))
}

pub(super) fn file_at(
    parent: &File,
    leaf: &OsStr,
    size: Option<u64>,
    _seeding: bool,
) -> io::Result<File> {
    let leaf = name(leaf)?;
    let open = |access| {
        opened(unsafe {
            libc::openat(
                parent.as_raw_fd(),
                leaf.as_ptr(),
                access | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                0o600,
            )
        })
    };
    let file = if size.is_some() {
        // Concurrent openat(O_CREAT) can return ENOENT on macOS. Use exclusive
        // creation or an ordinary open of the winner, both with O_NOFOLLOW.
        match open(libc::O_RDWR | libc::O_CREAT | libc::O_EXCL) {
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => open(libc::O_RDWR)?,
            result => result?,
        }
    } else {
        open(libc::O_RDONLY)?
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    if let Some(len) = size
        && metadata.len() != len
    {
        file.set_len(len)?;
    }
    Ok(file)
}

pub(super) fn file_for_move(parent: &File, leaf: &OsStr) -> io::Result<File> {
    file_at(parent, leaf, None, false)
}

pub(super) fn create_file(parent: &File, leaf: &OsStr) -> io::Result<File> {
    let leaf = name(leaf)?;
    opened(unsafe {
        libc::openat(
            parent.as_raw_fd(),
            leaf.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    })
}

pub(super) fn remove_at(parent: &File, leaf: &OsStr, directory: bool) -> io::Result<()> {
    let name = name(leaf)?;
    let flags = if directory { libc::AT_REMOVEDIR } else { 0 };
    // unlinkat never follows the terminal name; all parents were opened with O_NOFOLLOW.
    if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), flags) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Directory;
    use crate::testkit::TempFolder;
    use std::io::{Read, Write};
    use std::os::unix::fs::symlink;
    use std::{fs, path::PathBuf};

    #[test]
    fn concurrent_first_writes_keep_every_block() {
        use std::{os::unix::fs::FileExt, sync::Barrier, thread};

        const WRITERS: usize = 8;
        const BLOCK: usize = 64;
        let base = TempFolder::new();
        let root = Directory::open(&base.path()).unwrap();
        // Each fresh name exercises the first-create race between peer writers.
        for round in 0..32 {
            let relative = PathBuf::from(format!("piece-{round}.bin"));
            let ready = Barrier::new(WRITERS);
            thread::scope(|scope| {
                let workers: Vec<_> = (0..WRITERS)
                    .map(|index| {
                        let (root, relative, ready) = (&root, &relative, &ready);
                        scope.spawn(move || {
                            ready.wait();
                            root.file(relative, Some((WRITERS * BLOCK) as u64), false)?
                                .write_all_at(&[index as u8 + 1; BLOCK], (index * BLOCK) as u64)
                        })
                    })
                    .collect();
                for worker in workers {
                    worker.join().unwrap().unwrap_or_else(|error| {
                        panic!("concurrent first write, round {round}: {error}")
                    });
                }
            });
            let expected: Vec<_> = (0..WRITERS)
                .flat_map(|index| [index as u8 + 1; BLOCK])
                .collect();
            assert_eq!(fs::read(base.path().join(relative)).unwrap(), expected);
        }
    }

    #[test]
    fn reading_a_missing_file_does_not_create_it() {
        let base = TempFolder::new();
        let root = Directory::open(&base.path()).unwrap();
        let error = root.read_file(Path::new("missing.bin")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(fs::read_dir(base.path()).unwrap().count(), 0);
    }

    #[test]
    fn symlinks_and_hard_links_cannot_redirect_reads_or_writes() {
        let base = TempFolder::new();
        let outside = base.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let sentinel = outside.join("sentinel.bin");
        fs::write(&sentinel, b"keep this file").unwrap();
        let destination = base.path().join("destination");
        fs::create_dir(&destination).unwrap();
        let root = Directory::open_or_create(&destination).unwrap();
        symlink(&outside, destination.join("linked-directory")).unwrap();
        symlink(&sentinel, destination.join("linked-file")).unwrap();
        fs::hard_link(&sentinel, destination.join("hard-file")).unwrap();
        for path in ["linked-directory/sentinel.bin", "linked-file", "hard-file"] {
            assert!(root.read_file(Path::new(path)).is_err());
            assert!(root.file(Path::new(path), Some(1), false).is_err());
        }
        assert_eq!(fs::read(sentinel).unwrap(), b"keep this file");
    }

    #[test]
    fn opened_root_survives_replacement_without_following_a_new_symlink() {
        let base = TempFolder::new();
        let destination = base.path().join("destination");
        let outside = base.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let root = Directory::open_or_create(&destination).unwrap();
        root.file(Path::new("sub/a.bin"), Some(4), false)
            .unwrap()
            .write_all(b"test")
            .unwrap();
        let original = base.path().join("original");
        fs::rename(&destination, &original).unwrap();
        symlink(&outside, &destination).unwrap();
        assert!(Directory::open(&destination).is_err());
        assert!(Directory::open(&destination.join(".")).is_err());
        root.file(Path::new("sub/b.bin"), Some(3), false)
            .unwrap()
            .write_all(b"new")
            .unwrap();
        let mut bytes = Vec::new();
        root.read_file(Path::new("sub/a.bin"))
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes, b"test");
        assert_eq!(fs::read(original.join("sub/b.bin")).unwrap(), b"new");
        assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
    }
}
