use anyhow::{Context, Result, ensure};
use std::{
    fs::{File, OpenOptions},
    path::{Component, Path},
};
use tokio::fs::File as AsyncFile;
use tokio::io::{AsyncWriteExt, BufWriter};

/// Bound memory per worker while coalescing small network chunks into disk writes.
pub(crate) fn buffered(file: AsyncFile) -> BufWriter<AsyncFile> {
    BufWriter::with_capacity(256 * 1024, file)
}

/// Always wait for underlying I/O, even when flushing the outer buffer fails.
/// Call on success, cancellation and errors before a worker releases its range.
pub(crate) async fn flush_and_sync(file: &mut BufWriter<AsyncFile>) -> Result<()> {
    let flushed = file.flush().await;
    let synced = file.get_ref().sync_all().await;
    flushed?;
    synced?;
    Ok(())
}

/// Do not share write/delete access while a Windows transfer owns this file.
/// OPEN_REPARSE_POINT lets us inspect the opened object instead of following it.
fn options() -> OpenOptions {
    #[allow(unused_mut)]
    let mut options = OpenOptions::new();
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
        };
        options
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .share_mode(FILE_SHARE_READ);
    }
    options
}

fn check_file(file: &File, allow_hard_links: bool) -> Result<()> {
    ensure!(
        file.metadata()?.is_file(),
        "The download path is not a regular file"
    );
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, GetFileInformationByHandle,
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // The live File owns this borrowed handle; info is a correctly sized output.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(std::io::Error::last_os_error()).context("Cannot verify the download file");
        }
        ensure!(
            info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT == 0,
            "Refusing a linked or redirected partial download file"
        );
        ensure!(
            allow_hard_links || info.nNumberOfLinks == 1,
            "Refusing a partial download linked to another file. Restart this download."
        );
    }
    #[cfg(not(windows))]
    let _ = allow_hard_links;
    Ok(())
}

pub(crate) fn mark_internet(path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::io::Write;
        let mut name = path.as_os_str().to_os_string();
        name.push(":Zone.Identifier");
        let mut marker = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(std::path::PathBuf::from(name))
            .context(
                "Cannot mark this file as an Internet download. Choose a local NTFS folder.",
            )?;
        // Deliberately omit signed URLs and referrers from persistent metadata.
        marker.write_all(b"[ZoneTransfer]\r\nZoneId=3\r\n")?;
        marker.sync_all()?;
    }
    #[cfg(not(windows))]
    let _ = path;
    Ok(())
}

/// Replace the staging name, never truncate its existing target. In particular,
/// a stale publication hard link may still point to a file the user has renamed.
pub async fn fresh(path: &Path) -> Result<AsyncFile> {
    create_fresh(path, false).await
}

/// The merger hashes the same guarded output handle after its child exits.
pub(crate) async fn fresh_read_write(path: &Path) -> Result<AsyncFile> {
    create_fresh(path, true).await
}

async fn create_fresh(path: &Path, readable: bool) -> Result<AsyncFile> {
    let path = path.to_owned();
    let file = tokio::task::spawn_blocking(move || fresh_sync(&path, readable)).await??;
    Ok(AsyncFile::from_std(file))
}

fn fresh_sync(path: &Path, readable: bool) -> Result<File> {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("Cannot replace a temporary download file"),
    }
    let file = options()
        .read(readable)
        .write(true)
        .create_new(true)
        .open(path)?;
    check_file(&file, false)?;
    mark_internet(path)?;
    Ok(file)
}

pub async fn append_part(path: &Path) -> Result<AsyncFile> {
    let path = path.to_owned();
    let file = tokio::task::spawn_blocking(move || -> Result<File> {
        let file = options().create(true).append(true).open(&path)?;
        check_file(&file, false)?;
        mark_internet(&path)?;
        Ok(file)
    })
    .await??;
    Ok(AsyncFile::from_std(file))
}

pub async fn read_part(path: &Path) -> Result<AsyncFile> {
    let path = path.to_owned();
    let file = tokio::task::spawn_blocking(move || read_part_sync(&path)).await??;
    Ok(AsyncFile::from_std(file))
}

pub(crate) fn read_part_sync(path: &Path) -> Result<File> {
    let file = options().read(true).open(path)?;
    check_file(&file, false)?;
    Ok(file)
}

/// Compare the opened objects, not drive letters (which can alias volumes).
pub(crate) fn same_volume(first: &File, second: &File) -> Result<bool> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };
        fn volume(file: &File) -> Result<u32> {
            let mut info = BY_HANDLE_FILE_INFORMATION::default();
            // Each borrowed handle remains owned; info is a valid output buffer.
            if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
                return Err(std::io::Error::last_os_error())
                    .context("Cannot verify the download volume");
            }
            Ok(info.dwVolumeSerialNumber)
        }
        Ok(volume(first)? == volume(second)?)
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(first.metadata()?.dev() == second.metadata()?.dev())
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = (first, second);
        Ok(false)
    }
}

/// Keep the selected download folder open during finalization. Unlike explicit
/// saved-file move/delete operations, downloads already allow redirected folders
/// and network paths; open the selected directory target without widening access.
pub(crate) fn guard_download_parent(path: &Path) -> Result<File> {
    let parent = path.parent().context("Missing download folder")?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_READ, FILE_SHARE_WRITE,
        };
        options
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
    }
    let file = options
        .open(parent)
        .context("Cannot protect the download folder")?;
    ensure!(file.metadata()?.is_dir(), "The download folder changed");
    Ok(file)
}

/// One guarded handle owns both prefix verification and appending resumed FTP
/// bytes, denying competing writers/deletion throughout both operations.
pub(crate) async fn read_write_part(path: &Path) -> Result<AsyncFile> {
    let path = path.to_owned();
    let file = tokio::task::spawn_blocking(move || -> Result<File> {
        let file = options().read(true).write(true).open(&path)?;
        check_file(&file, false)?;
        mark_internet(&path)?;
        Ok(file)
    })
    .await??;
    Ok(AsyncFile::from_std(file))
}

/// Keep the cache directories open without delete sharing during a transfer.
/// A reparse point in app-owned cache storage must not redirect writes elsewhere.
pub async fn prepare_parts(base: &Path, root: &Path) -> Result<Vec<File>> {
    let base = base.to_owned();
    let root = root.to_owned();
    tokio::task::spawn_blocking(move || prepare_parts_sync(&base, &root)).await?
}
fn prepare_parts_sync(base: &Path, root: &Path) -> Result<Vec<File>> {
    fn open_directory(path: &Path) -> Result<File> {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
                FILE_SHARE_WRITE,
            };
            options
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
        }
        let file = options.open(path)?;
        let metadata = file.metadata()?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
            ensure!(
                metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0,
                "Refusing a linked download cache directory"
            );
        }
        ensure!(metadata.is_dir(), "Download cache path is not a directory");
        Ok(file)
    }
    let parent = open_directory(base)?;
    match std::fs::create_dir(root) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let child = open_directory(root)?;
    Ok(vec![parent, child])
}

/// Clear only files inside a guarded cache directory. Keep the directory handle
/// live so a junction swap cannot redirect reset operations outside this job.
pub(crate) fn reset_parts(base: &Path, root: &Path) -> Result<()> {
    ensure!(root.parent() == Some(base), "Invalid download cache path");
    let _guards = prepare_parts_sync(base, root)?;
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        ensure!(
            !entry.file_type()?.is_dir(),
            "Unexpected folder in the download cache"
        );
        std::fs::remove_file(entry.path()).context("Cannot reset a partial download file")?;
    }
    Ok(())
}

/// Delete only the flat, guarded cache we own. A recursive delete would also
/// erase unexpected folders and could follow a redirected cache parent.
pub(crate) fn discard_partial_files(
    paths: &crate::paths::AppPaths,
    job: &crate::model::Job,
) -> Result<()> {
    let root = paths.parts(&job.id)?;
    match std::fs::symlink_metadata(&root) {
        Ok(_) => {
            reset_parts(&paths.data.join("parts"), &root)?;
            // The guards have been released. Removing just the empty directory
            // cannot recurse into any replacement installed at this name.
            std::fs::remove_dir(&root).context("Cannot remove the download cache")?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("Cannot inspect the download cache"),
    }
    let staging = job.directory.join(format!(".idm-ex-{}.assembling", job.id));
    match std::fs::remove_file(staging) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("Cannot reset the temporary download"),
    }
}

/// Published files may have multiple hard links; never follow a reparse point.
pub(crate) fn read_completed(path: &Path) -> Result<File> {
    let file = options()
        .read(true)
        .open(path)
        .context("Cannot open the saved file for checksum verification")?;
    check_file(&file, true)?;
    Ok(file)
}

/// Delete the exact regular file opened for this action, never a replacement at
/// its pathname. Parent guards prevent junction swaps while the handle is live.
pub(crate) fn delete_completed(path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
        use windows_sys::Win32::Storage::FileSystem::{
            DELETE, FILE_DISPOSITION_INFO, FILE_READ_ATTRIBUTES, FileDispositionInfo,
            SetFileInformationByHandle,
        };
        let _parents = guard_parents(path)?;
        let file = options()
            .access_mode(DELETE | FILE_READ_ATTRIBUTES)
            .open(path)
            .context("Cannot open the saved file for deletion; it may be missing or in use")?;
        check_file(&file, true)?;
        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        // The owned handle has DELETE access. The buffer matches the selected
        // information class; Windows deletes this file when the handle closes.
        if unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle(),
                FileDispositionInfo,
                (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error())
                .context("Cannot delete the saved file; close any program using it and try again");
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        anyhow::bail!("Deleting saved files is only supported on Windows")
    }
}

/// Rename or move a finished file to `to`, never replacing a file already
/// there. Another drive needs a copy: it is checked against the original's
/// size before the original is deleted, and removed again if that fails.
pub(crate) fn move_completed(from: &Path, to: &Path) -> Result<()> {
    let _from = guard_parents(from)?;
    let _to = guard_parents(to)?;
    let size = read_completed(from)?.metadata()?.len();
    match move_new(from, to) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            anyhow::bail!(
                "{} already exists. Choose another name or folder.",
                to.display()
            )
        }
        // ERROR_NOT_SAME_DEVICE: the destination is on another drive.
        Err(error) if error.raw_os_error() == Some(17) => copy_completed(from, to, size),
        Err(error) => Err(error)
            .context("Cannot move the saved file; close any program using it and try again"),
    }
}

fn copy_completed(from: &Path, to: &Path, size: u64) -> Result<()> {
    let name = to
        .file_name()
        .context("Missing file name")?
        .to_string_lossy();
    let temporary = to.with_file_name(format!(".{name}.{}.moving", uuid::Uuid::new_v4()));
    let copied = (|| -> Result<()> {
        // CopyFileEx keeps alternate streams, including the downloaded-from-the-internet mark.
        std::fs::copy(from, &temporary).context("Cannot copy the saved file to the other drive")?;
        ensure!(
            std::fs::metadata(&temporary)?.len() == size,
            "The copy on the other drive is incomplete"
        );
        match move_new(&temporary, to) {
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                anyhow::bail!(
                    "{} already exists. Choose another name or folder.",
                    to.display()
                )
            }
            result => Ok(result?),
        }
    })();
    if let Err(error) = copied {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(error) = delete_completed(from) {
        let _ = std::fs::remove_file(to);
        return Err(error.context("The original could not be deleted, so the file was not moved"));
    }
    Ok(())
}

#[cfg(windows)]
fn move_new(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};
    let wide =
        |path: &Path| -> Vec<u16> { path.as_os_str().encode_wide().chain(Some(0)).collect() };
    let (from, to) = (wide(from), wide(to));
    // Without MOVEFILE_REPLACE_EXISTING or COPY_ALLOWED the move fails if the
    // destination exists, and also across drives.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}
#[cfg(not(windows))]
fn move_new(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::hard_link(from, to)?;
    std::fs::remove_file(from)
}

pub(crate) fn validate_path(path: &Path) -> Result<()> {
    ensure!(path.is_absolute(), "Choose an absolute local file path");
    for component in path.components() {
        match component {
            Component::Normal(name) => {
                let name = name.to_string_lossy();
                ensure!(
                    !name.contains(['\0', ':', '*', '?', '"', '\r', '\n'])
                        && !name.ends_with(['.', ' ']),
                    "File paths cannot contain wildcards, streams, or ambiguous names"
                );
            }
            Component::ParentDir | Component::CurDir => {
                anyhow::bail!("Use a normalized absolute path")
            }
            #[cfg(windows)]
            Component::Prefix(prefix) => ensure!(
                matches!(prefix.kind(), std::path::Prefix::Disk(_)),
                "This operation requires a local drive path, not a network or device path"
            ),
            _ => {}
        }
    }
    Ok(())
}

/// Pin the entire path to prevent a directory/junction swap during an operation.
pub(crate) fn guard_parents(path: &Path) -> Result<Vec<File>> {
    validate_path(path)?;
    let mut parents: Vec<_> = path
        .parent()
        .context("Missing parent folder")?
        .ancestors()
        .collect();
    parents.reverse();
    let mut guards = Vec::new();
    for parent in parents {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::Storage::FileSystem::*;
            options
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
        }
        let file = options
            .open(parent)
            .context("Cannot protect the file's folder during this operation")?;
        let meta = file.metadata()?;
        ensure!(meta.is_dir(), "The saved file folder changed");
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            ensure!(
                meta.file_attributes()
                    & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
                    == 0,
                "Using linked folders is unsupported. Use a regular local folder."
            );
        }
        guards.push(file);
    }
    Ok(guards)
}

#[cfg(all(test, windows))]
mod move_tests {
    use super::*;
    #[test]
    fn moves_never_replace_files_and_a_copy_is_checked_before_the_original_goes() {
        let temp = tempfile::tempdir().unwrap();
        let from = temp.path().join("report.pdf");
        let taken = temp.path().join("taken.pdf");
        std::fs::write(&from, b"report").unwrap();
        std::fs::write(&taken, b"keep").unwrap();
        let error = move_completed(&from, &taken).unwrap_err();
        assert!(error.to_string().contains("already exists"), "{error:#}");
        assert_eq!(std::fs::read(&taken).unwrap(), b"keep");
        // The other-drive path: copy, check the size, then delete the original.
        let size = std::fs::metadata(&from).unwrap().len();
        let error = copy_completed(&from, &taken, size).unwrap_err();
        assert!(error.to_string().contains("already exists"), "{error:#}");
        assert!(from.is_file());
        assert_eq!(std::fs::read(&taken).unwrap(), b"keep");
        let to = temp.path().join("moved.pdf");
        assert!(copy_completed(&from, &to, size + 1).is_err());
        assert!(from.is_file() && !to.exists());
        copy_completed(&from, &to, size).unwrap();
        assert!(!from.exists());
        assert_eq!(std::fs::read(&to).unwrap(), b"report");
        // No temporary copies are left behind.
        let names: Vec<_> = std::fs::read_dir(temp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
    }
}
