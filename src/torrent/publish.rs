//! A torrent downloads into a temporary folder beside its destination. Its files appear in
//! the download folder only when every piece has passed its hash check, and they never
//! replace anything already there.
use super::{File, Files};
use crate::{
    download_file, model::Job, store::Store, synchronization::FinishedFile,
    transfer_activity::Phase,
};
use anyhow::{Context, Result, ensure};
use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
    time::Duration,
};

/// Security software may still be reading a new file when the folder moves.
const MOVE_TRIES: u32 = 6;
const MOVE_RETRY: Duration = Duration::from_secs(1);

pub(super) fn staging(job: &Job) -> PathBuf {
    job.directory.join(format!(".idm-ex-{}.torrent", job.id))
}

fn is_missing(error: &std::io::Error) -> bool {
    matches!(error.kind(), ErrorKind::NotFound)
}

fn delete_file(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if !is_missing(&error) => {
            Err(error).with_context(|| format!("Cannot remove {}", path.display()))
        }
        _ => Ok(()),
    }
}

/// Removes the empty folders left above a deleted file, up to and not including `root`.
fn remove_empty_folders(root: &Path, file: &Path) {
    let mut folder = file.parent();
    while let Some(path) = folder {
        if path == root || !path.starts_with(root) || std::fs::remove_dir(path).is_err() {
            break;
        }
        folder = path.parent();
    }
}

/// A file that was not chosen can still exist, holding the pieces it shares with one that was.
fn remove_skipped(root: &Path, files: &Files) -> Result<()> {
    for file in files
        .list
        .iter()
        .filter(|file| !files.chosen.contains(&file.index))
    {
        let path = root.join(&file.saved_as);
        delete_file(&path)?;
        remove_empty_folders(root, &path);
    }
    Ok(())
}

fn check_complete(root: &Path, files: &Files) -> Result<()> {
    for file in files.chosen_files() {
        let path = root.join(&file.saved_as);
        let info = std::fs::symlink_metadata(&path)
            .with_context(|| format!("{} is missing. Resume to download it.", file.path))?;
        ensure!(
            info.is_file() && info.len() == file.len,
            "{} changed before saving. Resume to check it again.",
            file.path
        );
    }
    Ok(())
}

/// The only file in a torrent goes straight in the download folder. Anything more keeps the
/// torrent's own folder, so its files stay together.
fn lone_file(files: &Files) -> Option<&File> {
    match files.list.as_slice() {
        [file] if file.saved_as.components().count() == 1 => Some(file),
        _ => None,
    }
}

#[cfg(windows)]
fn move_folder(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};
    let wide =
        |path: &Path| -> Vec<u16> { path.as_os_str().encode_wide().chain(Some(0)).collect() };
    let (from, to) = (wide(from), wide(to));
    // Both strings end in a zero. Without MOVEFILE_REPLACE_EXISTING, nothing is replaced.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
fn move_folder(from: &Path, to: &Path) -> std::io::Result<()> {
    if to.exists() {
        return Err(ErrorKind::AlreadyExists.into());
    }
    std::fs::rename(from, to)
}

fn folder_candidate(directory: &Path, name: &str, index: u32) -> PathBuf {
    directory.join(match index {
        0 => name.to_owned(),
        index => format!("{name} ({index})"),
    })
}

async fn publish_folder(store: &Store, job: &mut Job, source: &Path) -> Result<PathBuf> {
    for index in 0..10000 {
        let path = folder_candidate(&job.directory, &job.name, index);
        if path.try_exists()? {
            continue;
        }
        // Write the intent first, so that a crash after the move still finds the folder.
        job.saved_path = Some(path.clone());
        store.save(job)?;
        let mut tries = 0;
        loop {
            match move_folder(source, &path) {
                Ok(()) => return Ok(path),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => break,
                Err(_) if tries + 1 < MOVE_TRIES => {
                    tries += 1;
                    tokio::time::sleep(MOVE_RETRY).await;
                }
                Err(error) => {
                    job.saved_path = None;
                    return Err(error)
                        .context("Cannot move the finished torrent to the download folder");
                }
            }
        }
    }
    anyhow::bail!("Too many folders have this name")
}

pub(super) async fn finish(store: &Store, job: &mut Job) -> Result<FinishedFile> {
    store.activity.set(&job.id, Phase::Saving);
    let files = store.torrent_files(&job.id)?;
    let root = staging(job);
    let check = (root.clone(), files.clone());
    tokio::task::spawn_blocking(move || {
        remove_skipped(&check.0, &check.1)?;
        check_complete(&check.0, &check.1)
    })
    .await??;
    let path = match lone_file(&files) {
        Some(file) => {
            let source = root.join(&file.saved_as);
            // A torrent's own name can differ from its file's, and lacks the extension.
            job.name = file.saved_as.to_string_lossy().into_owned();
            let path = crate::engine::publish_without_overwrite(store, job, &source).await?;
            delete_file(&source)?;
            std::fs::remove_dir(&root).context("Cannot remove the temporary torrent folder")?;
            path
        }
        None => publish_folder(store, job, &root).await?,
    };
    Ok(FinishedFile { path, guard: None })
}

/// Finishes the work of a run that stopped between publishing and saving the result.
pub(super) async fn recover(store: &Store, job: &mut Job) -> Result<Option<FinishedFile>> {
    let Some(path) = job.saved_path.clone() else {
        return Ok(None);
    };
    let root = staging(job);
    let files = store.torrent_files(&job.id)?;
    if path.try_exists()? {
        if !root.try_exists()? {
            return Ok(Some(FinishedFile { path, guard: None }));
        }
        if let Some(file) = lone_file(&files) {
            let source = root.join(&file.saved_as);
            if source.try_exists()? && same_file::is_same_file(&source, &path)? {
                delete_file(&source)?;
                std::fs::remove_dir(&root).context("Cannot remove the temporary torrent folder")?;
                return Ok(Some(FinishedFile { path, guard: None }));
            }
        }
    }
    job.saved_path = None;
    Ok(None)
}

/// Removes what an unfinished torrent left behind. The temporary folder is ours, and the
/// download folder's other contents are never touched.
pub(crate) fn discard(store: &Store, job: &Job) -> Result<()> {
    if job.torrent.is_none() {
        return Ok(());
    }
    match std::fs::remove_dir_all(staging(job)) {
        Err(error) if !is_missing(&error) => {
            return Err(error).context("Cannot remove the temporary torrent folder");
        }
        _ => {}
    }
    super::forget(store, job)
}

/// Deletes a finished torrent. A folder may also hold what the user put there, so only the
/// files this download saved go, and only folders that end up empty. A file that has
/// changed size is no longer the one that was downloaded, so it stays too.
pub(crate) fn delete_saved(store: &Store, job: &Job, path: &Path) -> Result<()> {
    let info = std::fs::symlink_metadata(path)
        .with_context(|| format!("Cannot find {}", path.display()))?;
    if !info.is_dir() {
        return download_file::delete_completed(path);
    }
    let files = store.torrent_files(&job.id)?;
    for file in files.chosen_files() {
        let target = path.join(&file.saved_as);
        match std::fs::symlink_metadata(&target) {
            Err(error) if is_missing(&error) => {}
            Err(error) => return Err(error.into()),
            Ok(info) if info.len() != file.len => {}
            Ok(_) => download_file::delete_completed(&target)?,
        }
        remove_empty_folders(path, &target);
    }
    match std::fs::remove_dir(path) {
        Ok(()) => Ok(()),
        // Something else is in the folder now, and it stays.
        Err(error) if error.kind() == ErrorKind::DirectoryNotEmpty => Ok(()),
        Err(error) if is_missing(&error) => Ok(()),
        Err(error) => Err(error).context("Cannot remove the torrent's folder"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(index: usize, path: &str) -> File {
        File {
            index,
            path: path.into(),
            saved_as: path.into(),
            len: 3,
        }
    }

    fn write(root: &Path, path: &str) {
        let target = root.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, b"abc").unwrap();
    }

    #[test]
    fn skipped_files_go_and_their_empty_folders_with_them() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("t");
        for path in ["keep/a.bin", "skip/b.bin", "keep/c.bin"] {
            write(&root, path);
        }
        let files = Files {
            list: vec![
                file(0, "keep/a.bin"),
                file(1, "skip/b.bin"),
                file(2, "keep/c.bin"),
            ],
            chosen: vec![0],
        };
        remove_skipped(&root, &files).unwrap();
        assert!(root.join("keep/a.bin").exists());
        assert!(!root.join("keep/c.bin").exists());
        assert!(!root.join("skip").exists());
        assert!(root.exists(), "the torrent's own folder stays");
    }

    #[test]
    fn a_missing_or_changed_file_stops_publishing() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(root, "a.bin");
        let files = Files {
            list: vec![file(0, "a.bin"), file(1, "b.bin")],
            chosen: vec![0, 1],
        };
        assert!(check_complete(root, &files).is_err());
        write(root, "b.bin");
        assert!(check_complete(root, &files).is_ok());
        std::fs::write(root.join("b.bin"), b"longer").unwrap();
        assert!(check_complete(root, &files).is_err());
    }

    #[test]
    fn only_a_lone_file_at_the_top_leaves_its_folder() {
        let lone = |paths: &[&str]| {
            let files = Files {
                list: paths
                    .iter()
                    .enumerate()
                    .map(|(i, path)| file(i, path))
                    .collect(),
                chosen: vec![0],
            };
            lone_file(&files).is_some()
        };
        assert!(lone(&["a.iso"]));
        assert!(!lone(&["folder/a.iso"]));
        assert!(!lone(&["a.iso", "b.txt"]));
    }

    #[test]
    fn moving_a_folder_never_replaces_anything() {
        let temp = tempfile::tempdir().unwrap();
        let (from, to) = (temp.path().join("from"), temp.path().join("to"));
        write(&from, "a.bin");
        write(&to, "b.bin");
        let error = move_folder(&from, &to).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::AlreadyExists, "{error}");
        assert!(from.join("a.bin").exists() && to.join("b.bin").exists());
        let fresh = temp.path().join("fresh");
        move_folder(&from, &fresh).unwrap();
        assert!(fresh.join("a.bin").exists() && !from.exists());
    }
}
