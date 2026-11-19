use super::{Directory, Storage};
use crate::{bitfield::Bitfield, resume, v2::InfoHashes};
use anyhow::{Context, Result, ensure};
use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;

#[derive(PartialEq, Eq)]
struct FileVersion {
    len: u64,
    modified: std::time::SystemTime,
    #[cfg(unix)]
    device_inode: (u64, u64),
    #[cfg(unix)]
    changed: (i64, i64),
    #[cfg(windows)]
    id: String,
}

impl FileVersion {
    fn read(file: std::fs::File) -> io::Result<Self> {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        let meta = file.metadata()?;
        Ok(Self {
            len: meta.len(),
            modified: meta.modified()?,
            #[cfg(unix)]
            device_inode: (meta.dev(), meta.ino()),
            #[cfg(unix)]
            changed: (meta.ctime(), meta.ctime_nsec()),
            #[cfg(windows)]
            id: super::native::identity(&file)?,
        })
    }
}

fn confirm_saved_files(
    storage: &Storage,
    versions: &[(usize, FileVersion)],
    check_cancel: impl Fn() -> io::Result<()>,
) -> Result<()> {
    const CHANGED: &str = "Saved torrent files changed during verification. Check them again.";
    for (index, before) in versions {
        check_cancel()?;
        // Reopen the published name. A cached piece handle can still read an old inode.
        let after = storage
            .native_file(*index, false)
            .and_then(FileVersion::read);
        ensure!(*before == after.context(CHANGED)?, CHANGED);
    }
    let named = Directory::open(&storage.root).and_then(|d| d.identity());
    ensure!(named.context(CHANGED)? == storage.directory_id()?, CHANGED);
    Ok(())
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Verification {
    pub verified_pieces: u32,
    pub damaged_pieces: u32,
    pub shared_pieces: u32,
    pub missing_files: usize,
    pub wrong_size_files: usize,
}

/// Read only the selected published files. The selected names and lengths must
/// agree with the cached, authenticated metadata before any payload is opened.
pub fn verify_saved(
    state_dir: &Path,
    hashes: InfoHashes,
    root: &Path,
    single_name: Option<String>,
    selected: &[(usize, PathBuf, u64)],
    cancel: &CancellationToken,
) -> Result<Verification> {
    let check_cancel = || {
        if cancel.is_cancelled() {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Verification cancelled",
            ))
        } else {
            Ok(())
        }
    };
    check_cancel()?;
    let meta = Arc::new(
        resume::load_info(state_dir, hashes, vec![])
            .context("Verified torrent metadata is unavailable offline")?,
    );
    ensure!(!selected.is_empty(), "No saved torrent files selected");
    let mut chosen = vec![false; meta.files.len()];
    let mut indices = Vec::with_capacity(selected.len());
    for (index, path, len) in selected {
        let file = meta
            .files
            .get(*index)
            .context("Saved torrent file index changed")?;
        ensure!(
            !chosen[*index] && !file.virtual_file && &file.path == path && file.len == *len,
            "Saved torrent file list does not match its metadata"
        );
        chosen[*index] = true;
        indices.push(*index);
    }
    let wanted = meta.wanted_pieces(Some(&indices))?;
    let mut storage =
        Storage::for_seeding(meta.clone(), root.to_owned(), single_name, Some(&indices))?;
    let mut report = Verification::default();
    match Directory::open(root) {
        Ok(directory) => storage.bind_directory(Arc::new(directory)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            report.missing_files = indices.len();
            return Ok(report);
        }
        Err(e) => return Err(e.into()),
    }
    let mut unavailable = Bitfield::new(meta.lengths.total_pieces());
    let mut shared = Bitfield::new(meta.lengths.total_pieces());
    let mut versions = Vec::with_capacity(indices.len());
    for (index, file) in meta.files.iter().enumerate() {
        check_cancel()?;
        if file.virtual_file {
            continue;
        }
        let mask = if chosen[index] {
            match storage
                .native_file(index, false)
                .and_then(FileVersion::read)
            {
                Ok(version) if version.len == file.len => {
                    versions.push((index, version));
                    continue;
                }
                Ok(_) => report.wrong_size_files += 1,
                Err(e) if e.kind() == io::ErrorKind::NotFound => report.missing_files += 1,
                Err(e) => {
                    return Err(e).with_context(|| format!("Cannot read {}", file.path.display()));
                }
            }
            &mut unavailable
        } else {
            &mut shared
        };
        for piece in file.pieces.clone() {
            mask.set(piece);
        }
    }
    for piece in wanted.iter_set() {
        check_cancel()?;
        if unavailable.get(piece) {
            continue;
        }
        if shared.get(piece) {
            report.shared_pieces += 1;
        } else if storage.check_piece(piece, check_cancel)? {
            report.verified_pieces += 1;
        } else {
            report.damaged_pieces += 1;
        }
    }
    confirm_saved_files(&storage, &versions, check_cancel)?;
    check_cancel()?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{TempFolder, meta_of, multi_file_torrent, single_file_torrent};
    use std::fs;

    #[cfg(unix)]
    #[test]
    fn saved_versions_detect_edits_replacements_and_a_moved_download_folder() {
        let temp = TempFolder::new();
        let root = temp.path().join("download");
        fs::create_dir(&root).unwrap();
        let path = root.join("saved");
        fs::write(&path, b"original").unwrap();
        let meta = meta_of(&single_file_torrent("saved", b"original", 16384));
        let storage = Storage::for_seeding(meta, root.clone(), None, Some(&[0])).unwrap();
        let snapshot = || {
            [(
                0,
                FileVersion::read(storage.native_file(0, false).unwrap()).unwrap(),
            )]
        };
        let before = snapshot();
        confirm_saved_files(&storage, &before, || Ok(())).unwrap();
        fs::write(&path, b"modified").unwrap();
        fs::File::open(&path)
            .unwrap()
            .set_modified(std::time::SystemTime::UNIX_EPOCH)
            .unwrap();
        assert!(confirm_saved_files(&storage, &before, || Ok(())).is_err());

        let before = snapshot();
        let replacement = root.join("new");
        fs::write(&replacement, b"new data").unwrap();
        fs::File::open(&replacement)
            .unwrap()
            .set_modified(before[0].1.modified)
            .unwrap();
        fs::rename(replacement, &path).unwrap();
        assert!(confirm_saved_files(&storage, &before, || Ok(())).is_err());

        let before = snapshot();
        fs::rename(&root, temp.path().join("moved")).unwrap();
        fs::create_dir(&root).unwrap();
        assert!(confirm_saved_files(&storage, &before, || Ok(())).is_err());
    }

    #[test]
    fn verifies_saved_files_without_creating_skipped_boundary_or_empty_files() {
        let temp = TempFolder::new();
        let root = temp.path();
        let data = vec![42; 16_385];
        let meta = meta_of(&multi_file_torrent(
            "set",
            &[
                (&["keep"], &data, false),
                (&["skip"], b"x", false),
                (&["empty"], b"", false),
            ],
            16_384,
        ));
        resume::save_info(&root, &meta).unwrap();
        let selected: Vec<_> = [0, 2]
            .into_iter()
            .map(|i| (i, meta.files[i].path.clone(), meta.files[i].len))
            .collect();
        let run = || {
            verify_saved(
                &root,
                meta.hashes,
                &root,
                None,
                &selected,
                &CancellationToken::new(),
            )
            .unwrap()
        };
        let keep = root.join(&meta.files[0].path);
        fs::create_dir_all(keep.parent().unwrap()).unwrap();
        fs::write(&keep, &data).unwrap();
        let result = run();
        assert_eq!(
            (
                result.verified_pieces,
                result.shared_pieces,
                result.missing_files
            ),
            (1, 1, 1)
        );
        fs::write(root.join(&meta.files[2].path), []).unwrap();
        fs::write(&keep, vec![43; data.len()]).unwrap();
        let result = run();
        assert_eq!(
            (
                result.damaged_pieces,
                result.shared_pieces,
                result.missing_files
            ),
            (1, 1, 0)
        );
        fs::write(&keep, b"short").unwrap();
        assert_eq!(run().wrong_size_files, 1);
        assert!(!root.join(&meta.files[1].path).exists());
    }

    #[test]
    fn verifies_renamed_file_and_requires_authentic_cached_info() {
        let temp = TempFolder::new();
        let root = temp.path();
        let meta = meta_of(&single_file_torrent("original", b"sample", 16384));
        let selected = [(0, meta.files[0].path.clone(), 6)];
        let cancel = CancellationToken::new();
        let run = || {
            verify_saved(
                &root,
                meta.hashes,
                &root,
                Some("renamed".into()),
                &selected,
                &cancel,
            )
        };
        assert!(run().is_err());
        resume::save_info(&root, &meta).unwrap();
        fs::write(root.join("renamed"), b"sample").unwrap();
        assert_eq!(run().unwrap().verified_pieces, 1);
        cancel.cancel();
        assert!(run().is_err());
        assert!(!root.join("original").exists());
    }
}
