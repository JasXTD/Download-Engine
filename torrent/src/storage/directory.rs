//! An open directory is the authority for torrent paths, including across the helper handoff.
use super::native;
use std::{
    ffi::OsStr,
    fs::File,
    io,
    path::{Component, Path},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Creation {
    Open,
    New,
    OpenOrCreate,
}

/// A live filesystem handle. A replacement at its old name cannot redirect child opens.
#[derive(Debug)]
pub struct Directory(File);

impl Directory {
    pub fn open(path: &Path) -> io::Result<Self> {
        native::open_directory(path, false).map(Self)
    }

    /// Acquire the handle used to publish or remove the directory after the helper stops.
    pub fn open_for_move(path: &Path) -> io::Result<Self> {
        native::open_directory(path, true).map(Self)
    }

    pub fn handle(&self) -> &File {
        &self.0
    }

    /// Only the selected parent may already exist. Never adopt an unclaimed staging name.
    pub fn create_new(path: &Path) -> io::Result<Self> {
        Self::create(path, Creation::New)
    }

    pub(super) fn open_or_create(path: &Path) -> io::Result<Self> {
        match Self::open(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Self::create(path, Creation::OpenOrCreate)
            }
            result => result,
        }
    }

    fn create(path: &Path, creation: Creation) -> io::Result<Self> {
        let name = path.file_name().ok_or(io::ErrorKind::InvalidInput)?;
        let parent = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
        let parent = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
        let parent = Self::open_or_create(parent)?;
        native::directory_at(&parent.0, name, creation).map(Self)
    }

    pub fn identity(&self) -> io::Result<String> {
        native::identity(&self.0)
    }

    pub fn verify(&self, expected: &str) -> io::Result<()> {
        if self.identity()? != expected {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "The torrent staging folder was replaced",
            ));
        }
        Ok(())
    }

    /// Every native lookup receives exactly one component relative to its checked parent.
    fn parent<'a>(&self, path: &'a Path, create: bool) -> io::Result<(File, &'a OsStr)> {
        let mut parts = path.components();
        let Some(Component::Normal(leaf)) = parts.next_back() else {
            return Err(io::ErrorKind::InvalidInput.into());
        };
        let mut parent = self.0.try_clone()?;
        let creation = if create {
            Creation::OpenOrCreate
        } else {
            Creation::Open
        };
        for part in parts {
            let Component::Normal(name) = part else {
                return Err(io::ErrorKind::InvalidInput.into());
            };
            parent = native::directory_at(&parent, name, creation)?;
        }
        Ok((parent, leaf))
    }

    pub fn read_file(&self, path: &Path) -> io::Result<File> {
        self.file(path, None, false)
    }

    /// Open the verified parent once for a subsequent identity-checked operation.
    pub fn file_parent<'a>(&self, path: &'a Path) -> io::Result<(Self, &'a OsStr)> {
        let (parent, name) = self.parent(path, false)?;
        Ok((Self(parent), name))
    }

    /// Create an Internet-origin file exclusively under this directory.
    pub fn create_file(&self, path: &Path) -> io::Result<File> {
        let (parent, name) = self.parent(path, false)?;
        native::create_file(&parent, name)
    }

    pub fn file_for_move(&self, path: &Path) -> io::Result<File> {
        let (parent, name) = self.parent(path, false)?;
        native::file_for_move(&parent, name)
    }

    pub(super) fn file(&self, path: &Path, size: Option<u64>, seeding: bool) -> io::Result<File> {
        let (parent, name) = self.parent(path, size.is_some())?;
        native::file_at(&parent, name, size, seeding)
    }

    /// Unlink just this name in its opened parent. Directory removal requires an empty folder.
    pub fn remove(&self, path: &Path, directory: bool) -> io::Result<()> {
        let (parent, name) = self.parent(path, false)?;
        native::remove_at(&parent, name, directory)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::TempFolder;
    use std::fs;

    #[test]
    fn only_a_new_directory_can_be_claimed_and_only_its_identity_can_be_resumed() {
        let base = TempFolder::new();
        let path = base.path().join("staging");
        let root = Directory::create_new(&path).unwrap();
        let id = root.identity().unwrap();
        Directory::open(&path).unwrap().verify(&id).unwrap();
        assert_eq!(
            Directory::create_new(&path).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        let old = base.path().join("original");
        #[cfg(windows)]
        assert!(
            fs::rename(&path, &old).is_err(),
            "the active directory is pinned"
        );
        drop(root);
        fs::rename(&path, &old).unwrap();
        let replacement = Directory::create_new(&path).unwrap();
        assert_eq!(
            replacement.verify(&id).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        Directory::open(&old).unwrap().verify(&id).unwrap();
    }

    #[test]
    fn linked_files_are_rejected_before_resizing_marking_or_seeding() {
        let base = TempFolder::new();
        let source = base.path().join("original.bin");
        fs::write(&source, b"unchanged").unwrap();
        let path = base.path().join("staging");
        let name = Path::new("shared.bin");
        fs::create_dir(&path).unwrap();
        fs::hard_link(&source, path.join(name)).unwrap();
        let root = Directory::open(&path).unwrap();
        assert!(root.file(name, Some(0), false).is_err());
        assert!(root.file(name, None, true).is_err());
        assert!(root.file_for_move(name).is_err());
        assert_eq!(fs::read(&source).unwrap(), b"unchanged");
        #[cfg(windows)]
        assert_eq!(
            fs::read(base.path().join("original.bin:Zone.Identifier"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
    }

    #[cfg(windows)]
    #[test]
    fn private_staging_has_a_protected_inheritable_owner_and_system_acl() {
        use std::{os::windows::ffi::OsStrExt, ptr};
        use windows_sys::Win32::{
            Foundation::LocalFree,
            Security::{Authorization::*, DACL_SECURITY_INFORMATION},
        };
        let base = TempFolder::new();
        let path = base.path().join("staging");
        let _root = Directory::create_new(&path).unwrap();
        let wide: Vec<_> = path.as_os_str().encode_wide().chain([0]).collect();
        let mut descriptor = ptr::null_mut();
        let mut string = ptr::null_mut();
        let mut len = 0;
        // These APIs allocate the descriptor and its UTF-16 rendering with LocalAlloc.
        let dacl = unsafe {
            let result = GetNamedSecurityInfoW(
                wide.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                &mut descriptor,
            );
            assert_eq!(result, 0, "{}", io::Error::from_raw_os_error(result as i32));
            let converted = ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor,
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &mut string,
                &mut len,
            );
            LocalFree(descriptor);
            assert_ne!(converted, 0);
            let value = String::from_utf16_lossy(std::slice::from_raw_parts(string, len as usize));
            LocalFree(string.cast());
            value
        };
        assert!(dacl.starts_with("D:P"), "{dacl}");
        assert_eq!(dacl.matches('(').count(), 2, "{dacl}");
        for ace in ["(A;OICI;FA;;;OW)", "(A;OICI;FA;;;SY)"] {
            assert!(dacl.contains(ace), "{dacl}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn root_and_nested_junctions_are_refused_for_data_and_cleanup() {
        use std::{os::windows::process::CommandExt, process::Command};
        let base = TempFolder::new();
        let other = base.path().join("other");
        fs::create_dir(&other).unwrap();
        fs::write(other.join("keep.bin"), b"keep").unwrap();
        let path = base.path().join("staging");
        let root = Directory::create_new(&path).unwrap();
        let junction = path.join("redirect");
        let result = Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&junction)
            .arg(&other)
            .creation_flags(0x0800_0000)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "Could not create temporary junction: {:?}",
            result.stderr
        );
        assert!(Directory::open(&junction).is_err());
        assert!(Directory::create_new(&junction).is_err());
        let relative = Path::new("redirect/keep.bin");
        assert!(root.file(relative, Some(0), false).is_err());
        assert!(root.read_file(relative).is_err());
        assert!(root.remove(relative, false).is_err());
        assert_eq!(fs::read(other.join("keep.bin")).unwrap(), b"keep");
        fs::remove_dir(junction).unwrap();
    }
}
