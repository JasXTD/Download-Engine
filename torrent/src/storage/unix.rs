//! Keep torrent file operations beneath an opened destination directory.
use parking_lot::Mutex;
use std::{
    ffi::CString,
    fs::{self, File, OpenOptions},
    io,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt},
        },
    },
    path::{Component, Path, PathBuf},
    sync::Arc,
};

pub(super) struct Root {
    path: PathBuf,
    directory: Mutex<Option<Arc<File>>>,
}

fn name(value: &std::ffi::OsStr) -> io::Result<CString> {
    CString::new(value.as_bytes()).map_err(|_| io::ErrorKind::InvalidInput.into())
}

fn opened(fd: libc::c_int) -> io::Result<File> {
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

impl Root {
    pub(super) fn new(path: PathBuf) -> Self {
        Self {
            path,
            directory: Mutex::new(None),
        }
    }
    fn directory(&self, create: bool) -> io::Result<Arc<File>> {
        let mut saved = self.directory.lock();
        if let Some(directory) = &*saved {
            return Ok(directory.clone());
        }
        if create {
            fs::create_dir_all(&self.path)?;
        }
        // A user-chosen destination may have symlinked ancestors. Resolve that
        // explicit root once; never resolve torrent-supplied descendants.
        let path = fs::canonicalize(&self.path)?;
        let directory = Arc::new(
            OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(path)?,
        );
        *saved = Some(directory.clone());
        Ok(directory)
    }
    pub(super) fn open(&self, relative: &Path, write: bool, len: u64) -> io::Result<File> {
        let mut components = Vec::new();
        for part in relative.components() {
            let Component::Normal(part) = part else {
                return Err(io::ErrorKind::InvalidInput.into());
            };
            components.push(name(part)?);
        }
        let leaf = components.pop().ok_or(io::ErrorKind::InvalidInput)?;
        let root = self.directory(write)?;
        let mut directory = root.try_clone()?;
        for part in components {
            if write && unsafe { libc::mkdirat(directory.as_raw_fd(), part.as_ptr(), 0o700) } < 0 {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::AlreadyExists {
                    return Err(error);
                }
            }
            directory = opened(unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    part.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            })?;
        }
        let access = if write {
            libc::O_RDWR | libc::O_CREAT
        } else {
            libc::O_RDONLY
        };
        let file = opened(unsafe {
            libc::openat(
                directory.as_raw_fd(),
                leaf.as_ptr(),
                access | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                0o600,
            )
        })?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        if write && metadata.len() != len {
            file.set_len(len)?;
        }
        Ok(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::TempFolder;
    use std::io::{Read, Write};
    use std::os::unix::fs::symlink;

    #[test]
    fn concurrent_first_writes_keep_every_block() {
        use std::{os::unix::fs::FileExt, sync::Barrier, thread};

        const WRITERS: usize = 8;
        const BLOCK: usize = 64;
        let base = TempFolder::new();
        let root = Root::new(base.path());
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
                            root.open(relative, true, (WRITERS * BLOCK) as u64)?
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
    fn symlinks_and_hard_links_cannot_redirect_reads_or_writes() {
        let base = TempFolder::new();
        let outside = base.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let sentinel = outside.join("sentinel.bin");
        fs::write(&sentinel, b"keep this file").unwrap();
        let destination = base.path().join("destination");
        fs::create_dir(&destination).unwrap();
        let root = Root::new(destination.clone());
        symlink(&outside, destination.join("linked-directory")).unwrap();
        symlink(&sentinel, destination.join("linked-file")).unwrap();
        fs::hard_link(&sentinel, destination.join("hard-file")).unwrap();
        for path in ["linked-directory/sentinel.bin", "linked-file", "hard-file"] {
            assert!(root.open(Path::new(path), false, 0).is_err());
            assert!(root.open(Path::new(path), true, 1).is_err());
        }
        assert_eq!(fs::read(sentinel).unwrap(), b"keep this file");
    }

    #[test]
    fn opened_root_survives_replacement_without_following_a_new_symlink() {
        let base = TempFolder::new();
        let destination = base.path().join("destination");
        let outside = base.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let root = Root::new(destination.clone());
        root.open(Path::new("sub/a.bin"), true, 4)
            .unwrap()
            .write_all(b"test")
            .unwrap();
        let original = base.path().join("original");
        fs::rename(&destination, &original).unwrap();
        symlink(&outside, &destination).unwrap();
        root.open(Path::new("sub/b.bin"), true, 3)
            .unwrap()
            .write_all(b"new")
            .unwrap();
        let mut bytes = Vec::new();
        root.open(Path::new("sub/a.bin"), false, 0)
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes, b"test");
        assert_eq!(fs::read(original.join("sub/b.bin")).unwrap(), b"new");
        assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
    }
}
