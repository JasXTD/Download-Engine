//! A torrent downloads into a temporary folder beside its destination. Its files appear in
//! the download folder only when every piece has passed its hash check, and they never
//! replace anything already there.
use super::{
    File, Files,
    receipts::{self, FileReceipt, Receipts},
    staging::Stage,
};
use crate::{
    download_file, model::Job, store::Store, synchronization::FinishedFile,
    transfer_activity::Phase,
};
use anyhow::{Context, Result, ensure};
use de_torrent::storage::{Directory, file_identity};
use rusqlite::params;
use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
    time::Duration,
};

/// Security software may still be reading a new file when the folder moves.
const MOVE_TRIES: u32 = 6;
const MOVE_RETRY: Duration = Duration::from_secs(1);

fn is_missing(error: &std::io::Error) -> bool {
    matches!(error.kind(), ErrorKind::NotFound)
}

fn remove_staged_file(root: &Directory, relative: &Path) -> Result<()> {
    if let Err(e) = root.remove(relative, false)
        && !is_missing(&e)
    {
        return Err(e.into());
    }
    remove_empty_folders(root, relative)
}

fn remove_empty_folders(root: &Directory, relative: &Path) -> Result<()> {
    for dir in relative
        .ancestors()
        .skip(1)
        .take_while(|p| !p.as_os_str().is_empty())
    {
        match root.remove(dir, true) {
            Ok(()) => {}
            Err(e) if is_missing(&e) => {}
            Err(e) if e.kind() == ErrorKind::DirectoryNotEmpty => break,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Skipped files can still hold boundary pieces shared with a chosen file.
fn remove_skipped(root: &Directory, files: &Files) -> Result<()> {
    let chosen: std::collections::HashSet<_> = files.chosen.iter().collect();
    for file in files
        .list
        .iter()
        .filter(|file| !chosen.contains(&file.index))
    {
        remove_staged_file(root, &file.saved_as)?;
    }
    Ok(())
}

fn check_complete(root: &Directory, files: &Files) -> Result<Receipts> {
    let mut receipts = Receipts::new();
    let record_children = lone_file(files).is_none();
    for file in files.chosen_files() {
        let actual = root
            .read_file(&file.saved_as)
            .with_context(|| format!("Cannot verify {} before saving", file.path))?;
        let metadata = actual.metadata()?;
        ensure!(
            metadata.len() == file.len,
            "{} changed before saving. Resume to check it again.",
            file.path
        );
        if record_children {
            receipts.insert(
                file.saved_as.clone(),
                FileReceipt::from_metadata(&actual, &metadata)?,
            );
        }
    }
    Ok(receipts)
}

/// A directory rename publishes every entry. Check membership as well as the
/// lengths of chosen leaves, and descend only into known ordinary directories.
fn check_membership(path: &Path, files: &Files) -> Result<()> {
    let mut expected = std::collections::HashMap::new();
    for file in files.chosen_files() {
        expected.insert(file.saved_as.clone(), false);
        for parent in file
            .saved_as
            .ancestors()
            .skip(1)
            .take_while(|p| !p.as_os_str().is_empty())
        {
            expected.insert(parent.to_owned(), true);
        }
    }
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        for entry in std::fs::read_dir(path.join(&relative))? {
            let entry = entry?;
            let child = relative.join(entry.file_name());
            let directory = expected
                .remove(&child)
                .context("Unexpected entry in torrent staging")?;
            let ty = entry.file_type()?;
            ensure!(
                (directory && ty.is_dir()) || (!directory && ty.is_file()),
                "Unexpected torrent entry type"
            );
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                ensure!(
                    entry.metadata()?.file_attributes()
                        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
                        == 0,
                    "Unexpected reparse point in torrent staging"
                );
            }
            if directory {
                pending.push(child);
            }
        }
    }
    ensure!(
        expected.is_empty(),
        "Torrent staging is missing a selected entry"
    );
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

fn candidate(job: &Job, index: u32, single: bool) -> PathBuf {
    if index == 0 {
        return job.directory.join(&job.name);
    }
    let name = Path::new(&job.name);
    let stem = if single {
        name.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&job.name)
    } else {
        &job.name
    };
    let ext = single
        .then(|| name.extension().and_then(|s| s.to_str()))
        .flatten();
    let name = match ext {
        Some(ext) => format!("{stem} ({index}).{ext}"),
        None => format!("{stem} ({index})"),
    };
    job.directory.join(name)
}

fn rename_entry(file: &std::fs::File, source: &Path, target: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        let _ = source;
        let parents = download_file::guard_parents(target)?;
        let parent = parents
            .last()
            .context("Missing torrent destination folder")?;
        de_torrent::storage::rename_into(
            parent,
            file,
            target
                .file_name()
                .context("Missing torrent destination name")?,
        )?;
        Ok(())
    }
    #[cfg(unix)]
    {
        #[cfg(test)]
        tests::before_folder_rename(target)?;
        crate::unix_files::rename_owned(file, source, target)
    }
}

async fn publish_entry(
    store: &Store,
    job: &mut Job,
    handle: &std::fs::File,
    source: &Path,
    single: bool,
) -> Result<PathBuf> {
    for index in 0..10000 {
        let path = candidate(job, index, single);
        if path.try_exists()? {
            continue;
        }
        // Recovery checks the published handle's identity as well as this intent.
        job.saved_path = Some(path.clone());
        store.save(job)?;
        for attempt in 1..=MOVE_TRIES {
            match rename_entry(handle, source, &path) {
                Ok(()) => return Ok(path),
                Err(e)
                    if e.downcast_ref::<std::io::Error>()
                        .is_some_and(|e| e.kind() == ErrorKind::AlreadyExists) =>
                {
                    break;
                }
                Err(e) if attempt == MOVE_TRIES => {
                    return Err(e)
                        .context("Cannot move the finished torrent to the download folder");
                }
                Err(_) => tokio::time::sleep(MOVE_RETRY).await,
            }
        }
    }
    anyhow::bail!("Too many downloads have this name")
}

pub(super) async fn finish(store: &Store, job: &mut Job, stage: &Stage) -> Result<FinishedFile> {
    store.activity.set(&job.id, Phase::Saving);
    let files = store.torrent_files(&job.id)?;
    #[cfg(windows)]
    let _parents = download_file::guard_parents(&stage.path)?;
    let root = stage.open_for_move()?;
    let stage_path = stage.path.clone();
    let (root, files, receipts) = tokio::task::spawn_blocking(move || -> Result<_> {
        remove_skipped(&root, &files)?;
        let receipts = check_complete(&root, &files)?;
        check_membership(&stage_path, &files)?;
        Ok((root, files, receipts))
    })
    .await??;
    let single = lone_file(&files);
    let leaf;
    let (source, handle) = if let Some(file) = single {
        job.name = file.saved_as.to_string_lossy().into_owned();
        leaf = root.file_for_move(&file.saved_as)?;
        (stage.path.join(&file.saved_as), &leaf)
    } else {
        (stage.path.clone(), root.handle())
    };
    {
        let mut conn = store.connect()?;
        let tx = conn.transaction()?;
        let count = tx.execute(
            "UPDATE torrent_staging SET published_id=?1 WHERE job_id=?2 AND directory_id=?3",
            params![file_identity(handle)?, job.id, stage.identity],
        )?;
        ensure!(count == 1, "The torrent staging record changed");
        if single.is_none() {
            receipts::save(&tx, &job.id, &receipts)?;
        }
        tx.commit()?;
    }
    let path = publish_entry(store, job, handle, &source, single.is_some()).await?;
    if single.is_some() {
        crate::file_relocation::delete_owned(root.handle(), &stage.path)?;
    }
    Ok(FinishedFile { path, guard: None })
}

#[cfg(unix)]
fn restore_move_source(stage: &Stage, files: &Files) -> Result<()> {
    if let Some(file) = lone_file(files) {
        let _root = match stage.open_for_move() {
            Err(e) if is_missing(&e) => return Ok(()),
            result => result?,
        };
        crate::unix_files::restore_interrupted_move(&stage.path.join(&file.saved_as), |entry| {
            Ok(stage.published.as_ref() == Some(&file_identity(entry)?))
        })
    } else {
        crate::unix_files::restore_interrupted_move(&stage.path, |entry| {
            Ok(file_identity(entry)? == stage.identity)
        })
    }
}

/// A name that appeared while the app was stopped is never proof of publication.
pub(super) async fn recover(store: &Store, job: &mut Job) -> Result<Option<FinishedFile>> {
    let Some(path) = job.saved_path.take() else {
        return Ok(None);
    };
    let Some(stage) = Stage::load(&store.connect()?, job)? else {
        return Ok(None);
    };
    let Some(expected) = &stage.published else {
        return Ok(None);
    };
    #[cfg(windows)]
    let _parents = download_file::guard_parents(&path)?;
    let files = store.torrent_files(&job.id)?;
    #[cfg(unix)]
    restore_move_source(&stage, &files)?;
    let single = lone_file(&files).is_some();
    let actual = if single {
        let name = path.file_name().context("Missing published filename")?;
        Directory::open(path.parent().context("Missing published folder")?)
            .and_then(|parent| parent.read_file(Path::new(name)))
            .and_then(|file| file_identity(&file))
    } else {
        Directory::open(&path).and_then(|dir| dir.identity())
    };
    let actual = match actual {
        Ok(identity) => identity,
        Err(e) if is_missing(&e) => return Ok(None),
        Err(e) => return Err(e).context("Cannot verify the published torrent"),
    };
    if &actual != expected {
        return Ok(None);
    }
    if single {
        match stage.open_for_move() {
            Ok(root) => crate::file_relocation::delete_owned(root.handle(), &stage.path)?,
            Err(e) if is_missing(&e) => {}
            Err(e) => return Err(e.into()),
        }
    }
    job.saved_path = Some(path.clone());
    Ok(Some(FinishedFile { path, guard: None }))
}

/// Delete only recorded torrent entries in the registered staging directory. Unexpected
/// files keep the folder nonempty, and an unregistered or replaced tree is never swept.
pub(crate) fn discard(store: &Store, job: &Job) -> Result<()> {
    let Some(torrent) = &job.torrent else {
        return Ok(());
    };
    let conn = store.connect()?;
    if let Some(stage) = Stage::load(&conn, job)? {
        #[cfg(unix)]
        if stage.published.is_some() {
            restore_move_source(&stage, &store.torrent_files(&job.id)?)?;
        }
        match stage.open_for_move() {
            Ok(root) => {
                if torrent.stage == super::Stage::Chosen {
                    for file in store.torrent_files(&job.id)?.list {
                        remove_staged_file(&root, &file.saved_as)?;
                    }
                }
                crate::file_relocation::delete_owned(root.handle(), &stage.path)?;
            }
            Err(e) if is_missing(&e) => {}
            Err(e) => {
                return Err(e).context("Cannot verify the temporary torrent folder for removal");
            }
        }
        conn.execute(
            "DELETE FROM torrent_staging WHERE job_id=?1 AND directory_id=?2",
            params![job.id, stage.identity],
        )?;
    }
    super::forget(store, job)
}

pub(crate) fn published_identity(store: &Store, job: &Job) -> Result<Option<String>> {
    Ok(Stage::load(&store.connect()?, job)?.and_then(|stage| stage.published))
}

/// Deletes a finished torrent. A folder may also hold what the user put there, so only the
/// files matching their publication records go, and only folders that end up empty.
pub(crate) fn delete_saved(store: &Store, job: &Job, path: &Path) -> Result<()> {
    #[cfg(windows)]
    let _parents = download_file::guard_parents(path)?;
    let files = store.torrent_files(&job.id)?;
    files.validate()?;
    let published = published_identity(store, job)?.context(
        "This torrent has no saved ownership record. Its files were kept; you can remove its history entry.",
    )?;
    if let Some(file) = lone_file(&files) {
        let parent = Directory::open(path.parent().context("Missing torrent destination")?)?;
        let leaf = Path::new(path.file_name().context("Missing torrent filename")?);
        return download_file::delete_relative_file(&parent, leaf, |actual| {
            Ok(actual.metadata()?.len() == file.len && file_identity(actual)? == published)
        });
    }
    let root = Directory::open_for_move(path)?;
    root.verify(&published)
        .context("The published torrent folder changed")?;
    let receipts = receipts::load(&store.connect()?, &job.id)?.context(
        "This torrent predates per-file ownership records. Its files were kept; you can remove its history entry.",
    )?;
    for file in files.chosen_files() {
        let Some(expected) = receipts.get(&file.saved_as) else {
            continue;
        };
        download_file::delete_relative_file(&root, &file.saved_as, |actual| {
            Ok(expected.size == file.len && expected.matches(actual)?)
        })?;
        remove_empty_folders(&root, &file.saved_as)?;
    }
    // Keep a folder containing personal files in place. The handle-based
    // removal below also handles an entry appearing after this check.
    if std::fs::read_dir(path)?.next().transpose()?.is_some() {
        return Ok(());
    }
    match crate::file_relocation::delete_owned(root.handle(), path) {
        Ok(()) => Ok(()),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == ErrorKind::DirectoryNotEmpty) =>
        {
            Ok(())
        }
        Err(error) => Err(error).context("Cannot remove the torrent's folder"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn move_folder(from: &Path, to: &Path) -> std::io::Result<()> {
        let root = Directory::open_for_move(from)?;
        rename_entry(root.handle(), from, to).map_err(|e| {
            let kind = e
                .downcast_ref::<std::io::Error>()
                .map_or(ErrorKind::Other, |e| e.kind());
            std::io::Error::new(kind, e)
        })
    }

    #[cfg(unix)]
    std::thread_local! {
        static CREATE_DESTINATION_BEFORE_RENAME: std::cell::Cell<bool> = const {
            std::cell::Cell::new(false)
        };
    }

    #[cfg(unix)]
    pub(super) fn before_folder_rename(to: &Path) -> std::io::Result<()> {
        if CREATE_DESTINATION_BEFORE_RENAME.replace(false) {
            std::fs::create_dir(to)?;
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn a_destination_created_during_publication_is_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let (from, to) = (temp.path().join("from"), temp.path().join("to"));
        write(&from, "a.bin");
        assert!(!to.exists());
        CREATE_DESTINATION_BEFORE_RENAME.set(true);
        let error = move_folder(&from, &to).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::AlreadyExists, "{error}");
        assert_eq!(std::fs::read(from.join("a.bin")).unwrap(), b"abc");
        assert!(to.is_dir());
        assert_eq!(std::fs::read_dir(&to).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn destination_symlinks_are_preserved() {
        for dangling in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let (from, to) = (temp.path().join("from"), temp.path().join("to"));
            let target = temp.path().join("target");
            write(&from, "a.bin");
            if !dangling {
                write(&target, "keep.bin");
            }
            std::os::unix::fs::symlink(&target, &to).unwrap();
            let error = move_folder(&from, &to).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::AlreadyExists, "{error}");
            assert_eq!(std::fs::read(from.join("a.bin")).unwrap(), b"abc");
            assert_eq!(std::fs::read_link(&to).unwrap(), target);
            if dangling {
                assert!(!target.exists());
            } else {
                assert_eq!(std::fs::read(target.join("keep.bin")).unwrap(), b"abc");
                assert!(!target.join("a.bin").exists());
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn source_symlinks_and_regular_files_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let (from, to) = (temp.path().join("from"), temp.path().join("to"));
        let target = temp.path().join("target");
        write(&target, "keep.bin");
        std::os::unix::fs::symlink(&target, &from).unwrap();
        assert!(move_folder(&from, &to).is_err());
        assert_eq!(std::fs::read_link(&from).unwrap(), target);
        assert_eq!(std::fs::read(target.join("keep.bin")).unwrap(), b"abc");
        assert!(!to.exists());

        let file = temp.path().join("file.bin");
        std::fs::write(&file, b"keep").unwrap();
        assert!(move_folder(&file, &to).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"keep");
        assert!(!to.exists());
    }

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
        remove_skipped(&Directory::open(&root).unwrap(), &files).unwrap();
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
        let directory = Directory::open(root).unwrap();
        assert!(check_complete(&directory, &files).is_err());
        write(root, "b.bin");
        assert!(check_complete(&directory, &files).is_ok());
        std::fs::write(root.join("b.bin"), b"longer").unwrap();
        assert!(check_complete(&directory, &files).is_err());
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
