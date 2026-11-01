use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct FileKey {
    volume: u64,
    index: u64,
    created: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Snapshot {
    pub key: FileKey,
    size: u64,
    modified: u64,
    #[serde(default)]
    pub directory: bool,
}
impl Snapshot {
    pub fn read(file: &File) -> Result<Self> {
        ensure!(
            file.metadata()?.is_file() || file.metadata()?.is_dir(),
            "File operation requires a regular file or folder"
        );
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::*;
            let mut info = BY_HANDLE_FILE_INFORMATION::default();
            if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            ensure!(
                info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT == 0,
                "Refusing a linked or redirected file"
            );
            let time = |t: windows_sys::Win32::Foundation::FILETIME| {
                (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)
            };
            Ok(Self {
                key: FileKey {
                    volume: u64::from(info.dwVolumeSerialNumber),
                    index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
                    created: time(info.ftCreationTime),
                },
                size: (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow),
                modified: time(info.ftLastWriteTime),
                directory: file.metadata()?.is_dir(),
            })
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let m = file.metadata()?;
            Ok(Self {
                key: FileKey {
                    volume: m.dev(),
                    index: m.ino(),
                    created: 0,
                },
                size: m.len(),
                modified: (m.mtime() as u64)
                    .wrapping_mul(1_000_000_000)
                    .wrapping_add(m.mtime_nsec() as u64),
                directory: m.is_dir(),
            })
        }
    }
}

pub(super) fn open_owned(path: &Path, create: bool) -> Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(create).create_new(create);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::{
            Foundation::{GENERIC_READ, GENERIC_WRITE},
            Storage::FileSystem::*,
        };
        options
            .access_mode(GENERIC_READ | DELETE | if create { GENERIC_WRITE } else { 0 })
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options
        .open(path)
        .with_context(|| format!("Cannot own file {}", path.display()))?;
    Snapshot::read(&file)?;
    ensure!(file.metadata()?.is_file(), "Expected a regular file");
    Ok(file)
}

pub(super) fn open_directory(path: &Path, writable: bool) -> Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_DIRECTORY);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::{
            Foundation::{GENERIC_READ, GENERIC_WRITE},
            Storage::FileSystem::*,
        };
        options
            .access_mode(GENERIC_READ | DELETE | if writable { GENERIC_WRITE } else { 0 })
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
    }
    #[cfg(not(windows))]
    let _ = writable;
    let file = options
        .open(path)
        .with_context(|| format!("Cannot own folder {}", path.display()))?;
    ensure!(
        Snapshot::read(&file)?.directory,
        "Expected a regular folder"
    );
    Ok(file)
}

pub(super) fn same_name_ignoring_case(from: &Path, to: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};
        let from: Vec<u16> = from.as_os_str().encode_wide().collect();
        let to: Vec<u16> = to.as_os_str().encode_wide().collect();
        unsafe {
            CompareStringOrdinal(
                from.as_ptr(),
                from.len() as i32,
                to.as_ptr(),
                to.len() as i32,
                1,
            ) == CSTR_EQUAL
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (from, to);
        false
    }
}

pub(super) fn rename_owned(file: &File, from: &Path, target: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::{ffi::OsStrExt, io::AsRawHandle};
        use windows_sys::Win32::Storage::FileSystem::*;
        let _ = from;
        let name: Vec<u16> = target.as_os_str().encode_wide().collect();
        ensure!(
            name.len() <= 32767 && !name.contains(&0),
            "Invalid destination path"
        );
        let size = std::mem::offset_of!(FILE_RENAME_INFO, FileName) + name.len() * 2;
        // u64 storage provides the Windows x64 structure's required alignment.
        let mut storage = vec![0u64; size.div_ceil(8)];
        let info = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();
        unsafe {
            (*info).FileNameLength = (name.len() * 2) as u32;
            std::ptr::copy_nonoverlapping(
                name.as_ptr(),
                std::ptr::addr_of_mut!((*info).FileName).cast::<u16>(),
                name.len(),
            );
            // ReplaceIfExists remains false. The open handle prevents source
            // replacement, and the caller pins the destination's parent chain.
            if SetFileInformationByHandle(
                file.as_raw_handle(),
                FileRenameInfo,
                info.cast(),
                size as u32,
            ) == 0
            {
                return Err(std::io::Error::last_os_error())
                    .context("Cannot rename the file without replacing another file");
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        crate::unix_files::rename_owned(file, from, target)
    }
}

pub(super) fn delete_owned(file: &File, path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        let _ = path;
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::*;
        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        if unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle(),
                FileDispositionInfo,
                (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        crate::unix_files::delete_owned(file, path)
    }
}

pub(super) fn check_copy(file: &File) -> Result<()> {
    #[cfg(unix)]
    let _ = file;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::*;
        let attributes = file.metadata()?.file_attributes();
        // Never materialize EFS-protected content as a plaintext staging file.
        ensure!(
            attributes & FILE_ATTRIBUTE_ENCRYPTED == 0,
            "Moving an encrypted file to another drive is unsupported; original preserved"
        );
        ensure!(
            attributes & FILE_ATTRIBUTE_READONLY == 0,
            "Remove the file's read-only attribute before moving it to another drive"
        );
    }
    Ok(())
}

pub(super) fn copy_metadata(source: &File, target: &File) -> Result<()> {
    #[cfg(unix)]
    {
        let metadata = source.metadata()?;
        let times = std::fs::FileTimes::new()
            .set_accessed(metadata.accessed()?)
            .set_modified(metadata.modified()?);
        target.set_permissions(metadata.permissions())?;
        target.set_times(times)?;
        if metadata.is_file() {
            target.sync_all()?;
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::*;
        let mut info = FILE_BASIC_INFO::default();
        let size = std::mem::size_of::<FILE_BASIC_INFO>() as u32;
        if unsafe {
            GetFileInformationByHandleEx(
                source.as_raw_handle(),
                FileBasicInfo,
                (&mut info as *mut FILE_BASIC_INFO).cast(),
                size,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        // Keep the staging file's creation identity, but retain content times
        // and portable attributes. Compression/sparsity are filesystem policy.
        info.CreationTime = 0;
        info.ChangeTime = 0;
        info.FileAttributes &= FILE_ATTRIBUTE_HIDDEN
            | FILE_ATTRIBUTE_SYSTEM
            | FILE_ATTRIBUTE_ARCHIVE
            | FILE_ATTRIBUTE_NORMAL
            | FILE_ATTRIBUTE_NOT_CONTENT_INDEXED
            | FILE_ATTRIBUTE_DIRECTORY;
        if info.FileAttributes == 0 {
            info.FileAttributes = FILE_ATTRIBUTE_NORMAL;
        }
        if unsafe {
            SetFileInformationByHandle(
                target.as_raw_handle(),
                FileBasicInfo,
                (&info as *const FILE_BASIC_INFO).cast(),
                size,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        if !target.metadata()?.is_dir() {
            target.sync_all()?;
        }
    }
    Ok(())
}

/// Stream every named NTFS data stream too, including origin/security metadata.
pub(super) fn copy_streams(
    source: &Path,
    target: &Path,
    cancel: &CancellationToken,
    copy: impl Fn(&mut File, &mut File, &CancellationToken) -> Result<()>,
) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt};
        use windows_sys::Win32::{
            Foundation::{ERROR_HANDLE_EOF, HANDLE, INVALID_HANDLE_VALUE},
            Storage::FileSystem::*,
        };
        struct Search(HANDLE);
        impl Drop for Search {
            fn drop(&mut self) {
                unsafe {
                    FindClose(self.0);
                }
            }
        }
        let wide: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut data = WIN32_FIND_STREAM_DATA::default();
        let handle = unsafe {
            FindFirstStreamW(
                wide.as_ptr(),
                FindStreamInfoStandard,
                (&mut data as *mut WIN32_FIND_STREAM_DATA).cast(),
                0,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_HANDLE_EOF as i32) {
                return Ok(());
            }
            return Err(error).context("Cannot enumerate the original file's data streams");
        }
        let search = Search(handle);
        loop {
            cancelled(cancel)?;
            let end = data
                .cStreamName
                .iter()
                .position(|c| *c == 0)
                .context("Invalid data stream name")?;
            let stream = String::from_utf16(&data.cStreamName[..end])?;
            if stream != "::$DATA" {
                ensure!(
                    stream.starts_with(':')
                        && stream.ends_with(":$DATA")
                        && !stream.contains(['/', '\\', '\0']),
                    "Invalid data stream name"
                );
                let mut from = source.as_os_str().to_owned();
                from.push(&stream);
                let mut to = target.as_os_str().to_owned();
                to.push(&stream);
                let mut input = std::fs::OpenOptions::new()
                    .read(true)
                    .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
                    .open(from)?;
                let mut output = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
                    .open(to)
                    .context(
                        "Destination cannot preserve the file's data streams; original preserved",
                    )?;
                copy(&mut input, &mut output, cancel)?;
                ensure!(
                    digest(&mut input, cancel)? == digest(&mut output, cancel)?,
                    "Copied data stream failed checksum verification; original preserved"
                );
            }
            if unsafe {
                FindNextStreamW(search.0, (&mut data as *mut WIN32_FIND_STREAM_DATA).cast())
            } == 0
            {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(ERROR_HANDLE_EOF as i32) {
                    break;
                }
                return Err(error.into());
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (source, target, cancel, copy);
        Ok(())
    }
}
