//! Folder moves use the same durable intent as single files. Cross-volume
//! copies are verified before publication; cleanup uses recorded file IDs.
use super::*;
use std::collections::{BTreeSet, VecDeque};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Entry {
    relative: PathBuf,
    source: Snapshot,
    target: Option<Snapshot>,
    hash: Option<[u8; 32]>,
}

fn relative(path: &Path) -> Result<()> {
    ensure!(
        path.components().count() > 0
            && path
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_))),
        "Invalid folder move path"
    );
    Ok(())
}

// Keep per-file checkpoints small. Rewriting a whole 10,000-file inventory
// after every copied file would turn a folder move into gigabytes of DB writes.
fn save_entry(store: &Store, intent: &Intent, index: usize) -> Result<()> {
    let entry = &intent.tree.as_ref().context("Missing folder inventory")?[index];
    store.connect()?.execute("INSERT INTO file_relocation_items(job_id,ordinal,data) VALUES(?1,?2,?3) ON CONFLICT(job_id,ordinal) DO UPDATE SET data=excluded.data",params![intent.job,i64::try_from(index)?,serde_json::to_string(entry)?])?;
    Ok(())
}

/// Hold every original file and directory until publication or cancellation.
/// Enumeration never follows junctions or symlinks.
fn collect(root: &Path, cancel: &CancellationToken) -> Result<(Vec<Entry>, Vec<Option<File>>)> {
    let mut pending = VecDeque::from([PathBuf::new()]);
    let mut entries = Vec::new();
    let mut handles = Vec::new();
    let mut path_bytes = 0usize;
    while let Some(parent) = pending.pop_front() {
        for child in std::fs::read_dir(root.join(&parent))? {
            cancelled(cancel)?;
            let child = child?;
            let path = parent.join(child.file_name());
            relative(&path)?;
            path_bytes += path.as_os_str().len();
            ensure!(
                entries.len() < 20_000 && path_bytes <= 8 * 1024 * 1024,
                "This folder is too large to move safely in the app"
            );
            let meta = child.file_type()?;
            ensure!(
                !meta.is_symlink(),
                "Folder contains a linked item; original preserved"
            );
            let file = if meta.is_dir() {
                handles::open_directory(&child.path(), false)?
            } else {
                open_owned(&child.path(), false)?
            };
            let snapshot = Snapshot::read(&file)?;
            if snapshot.directory {
                pending.push_back(path.clone());
            }
            entries.push(Entry {
                relative: path,
                source: snapshot,
                target: None,
                hash: None,
            });
            handles.push(Some(file));
        }
    }
    Ok((entries, handles))
}

fn current_names(root: &Path) -> Result<BTreeSet<PathBuf>> {
    let mut pending = VecDeque::from([PathBuf::new()]);
    let mut names = BTreeSet::new();
    while let Some(parent) = pending.pop_front() {
        for child in std::fs::read_dir(root.join(parent))? {
            let child = child?;
            let path = child.path().strip_prefix(root)?.to_owned();
            ensure!(
                names.len() < 20_000,
                "Folder contents changed during the move"
            );
            ensure!(
                !child.file_type()?.is_symlink(),
                "Folder contents changed during the move"
            );
            if child.file_type()?.is_dir() {
                pending.push_back(path.clone());
            }
            names.insert(path);
        }
    }
    Ok(names)
}

fn matches(snapshot: &Snapshot, expected: &Snapshot) -> bool {
    if expected.directory {
        snapshot.directory && snapshot.key == expected.key
    } else {
        snapshot == expected
    }
}

/// Delete only recorded identities, bottom-up. Unexpected additions keep their
/// parent folder in place and produce a recoverable error, never a recursive delete.
fn cleanup(root: &Path, root_key: &FileKey, entries: &[Entry], target: bool) -> Result<()> {
    let directory = handles::open_directory(root, false)?;
    ensure!(
        &Snapshot::read(&directory)?.key == root_key,
        "Folder changed; cleanup preserved it"
    );
    let mut owned = Vec::new();
    // Parents precede their children in the inventory. Keep each directory
    // pinned while opening its descendants; a recovery must not traverse a
    // substituted junction between checking a parent and deleting a child.
    for entry in entries {
        relative(&entry.relative)?;
        let expected = if target {
            entry.target.as_ref()
        } else {
            Some(&entry.source)
        };
        let Some(expected) = expected else { continue };
        let path = root.join(&entry.relative);
        if !path.try_exists()? {
            continue;
        }
        let file = if expected.directory {
            handles::open_directory(&path, false)?
        } else {
            open_owned(&path, false)?
        };
        let actual = Snapshot::read(&file)?;
        // An interrupted staging write may have a different length, but must
        // still be our exact file. Originals must retain all recorded metadata.
        ensure!(
            if target {
                actual.key == expected.key
            } else {
                matches(&actual, expected)
            },
            "A folder item changed; cleanup preserved it"
        );
        owned.push((path, file));
    }
    while let Some((path, file)) = owned.pop() {
        delete_owned(&file, &path)?;
    }
    delete_owned(&directory, root)
}

pub(super) fn relocate(
    store: &Store,
    job: &Job,
    target: PathBuf,
    name: String,
    destination: PathBuf,
    cancel: &CancellationToken,
    force_copy: bool,
) -> Result<()> {
    #[cfg(test)]
    let _hook = tests::enter(&job.id);
    #[cfg(test)]
    let force_copy = force_copy || tests::force_copy();
    ensure!(
        job.status == Status::Complete && job.torrent.is_some(),
        "Only completed torrent folders can be moved"
    );
    ensure!(
        !is_pending(store, &job.id)?,
        "A previous move needs recovery"
    );
    let source = job.saved_path.clone().context("Missing torrent folder")?;
    ensure!(
        source != target && !target.starts_with(&source),
        "Choose a destination outside this torrent folder"
    );
    let _source_parents = download_file::guard_parents(&source)?;
    let target_parents = download_file::guard_parents(&target)?;
    ensure!(
        !target.try_exists()? || handles::same_name_ignoring_case(&source, &target),
        "Destination already exists; choose another name or folder"
    );
    let original = handles::open_directory(&source, false)?;
    let source_snapshot = Snapshot::read(&original)?;
    let copied = force_copy
        || !download_file::same_volume(
            &original,
            target_parents
                .last()
                .context("Missing destination folder")?,
        )?;
    let (entries, mut originals) = collect(&source, cancel)?;
    if copied {
        handles::check_copy(&original)?;
        for file in originals.iter().flatten() {
            handles::check_copy(file)?;
        }
    } else {
        // The directory handle owns an atomic rename. Windows cannot rename a
        // tree while its children are held without delete sharing.
        originals.clear();
    }
    let mut staging = None;
    if copied {
        let path = target.with_file_name(format!(".de-relocate-{}.moving", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path)?;
        let file = handles::open_directory(&path, true)?;
        staging = Some((path, file));
    }
    let mut intent = Intent {
        job: job.id.clone(),
        source,
        target,
        source_snapshot: source_snapshot.clone(),
        temporary: staging
            .as_ref()
            .map(|(path, file)| Ok::<_, anyhow::Error>((path.clone(), Snapshot::read(file)?.key)))
            .transpose()?,
        target_snapshot: (!copied).then_some(source_snapshot),
        target_hash: None,
        name,
        directory: destination,
        committed: false,
        tree: Some(entries),
    };
    let inserted = store.connect()?.execute(
        "INSERT INTO file_relocations(job_id,data) VALUES(?1,?2)",
        params![intent.job, serde_json::to_string(&intent)?],
    );
    if let Err(error) = inserted {
        if let Some((path, directory)) = &staging {
            delete_owned(directory, path)?;
        }
        return Err(error.into());
    }
    let mut published = false;
    let result = (|| -> Result<()> {
        if let Some((stage, directory)) = &staging {
            for (index, original) in originals.iter_mut().enumerate() {
                cancelled(cancel)?;
                let entry = &intent.tree.as_ref().unwrap()[index];
                let from = intent.source.join(&entry.relative);
                let to = stage.join(&entry.relative);
                let source = original.as_mut().context("Missing original handle")?;
                let mut created = if entry.source.directory {
                    std::fs::create_dir(&to)?;
                    handles::open_directory(&to, true)?
                } else {
                    open_owned(&to, true)?
                };
                intent.tree.as_mut().unwrap()[index].target = Some(Snapshot::read(&created)?);
                save_entry(store, &intent, index)?;
                if !source.metadata()?.is_dir() {
                    copy_stream(source, &mut created, cancel)?;
                    let original_hash = digest(source, cancel)?;
                    ensure!(
                        original_hash == digest(&mut created, cancel)?,
                        "Copied torrent file failed checksum verification; original preserved"
                    );
                    intent.tree.as_mut().unwrap()[index].hash = Some(original_hash.1);
                }
                handles::copy_streams(&from, &to, cancel, copy_stream)?;
                handles::copy_metadata(source, &created)?;
                intent.tree.as_mut().unwrap()[index].target = Some(Snapshot::read(&created)?);
                save_entry(store, &intent, index)?;
            }
            ensure!(
                current_names(&intent.source)?
                    == intent
                        .tree
                        .as_ref()
                        .unwrap()
                        .iter()
                        .map(|entry| entry.relative.clone())
                        .collect(),
                "Folder contents changed; original preserved"
            );
            handles::copy_streams(&intent.source, stage, cancel, copy_stream)?;
            handles::copy_metadata(&original, directory)?;
            intent.target_snapshot = Some(Snapshot::read(directory)?);
            save(store, &intent)?;
            cancelled(cancel)?;
            rename_owned(directory, stage, &intent.target)?;
        } else {
            cancelled(cancel)?;
            rename_owned(&original, &intent.source, &intent.target)?;
        }
        published = true;
        #[cfg(test)]
        tests::after_publish(&intent.job);
        commit(store, &mut intent)?;
        Ok(())
    })();
    drop(originals);
    drop(original);
    drop(staging);
    if result.is_ok() {
        if copied {
            cleanup(
                &intent.source,
                &intent.source_snapshot.key,
                intent.tree.as_ref().unwrap(),
                false,
            )?;
        }
        clear(store, &intent.job)?;
    } else if !published {
        if let Some((stage, key)) = &intent.temporary {
            cleanup(stage, key, intent.tree.as_ref().unwrap(), true)?;
        }
        clear(store, &intent.job)?;
    }
    result
}

pub(super) fn recover(store: &Store, mut intent: Intent, cancel: &CancellationToken) -> Result<()> {
    {
        let conn = store.connect()?;
        let mut query = conn.prepare(
            "SELECT ordinal,data FROM file_relocation_items WHERE job_id=?1 ORDER BY ordinal",
        )?;
        for row in query.query_map([&intent.job], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })? {
            let (index, data) = row?;
            let update: Entry = serde_json::from_str(&data)?;
            let index = usize::try_from(index)?;
            let entry = intent
                .tree
                .as_mut()
                .and_then(|tree| tree.get_mut(index))
                .context("Invalid folder checkpoint")?;
            ensure!(
                entry.relative == update.relative && entry.source == update.source,
                "Folder checkpoint changed; recovery preserved all files"
            );
            *entry = update;
        }
    }
    let _source_parents = download_file::guard_parents(&intent.source)?;
    let _target_parents = download_file::guard_parents(&intent.target)?;
    let entries = intent.tree.as_ref().context("Missing folder inventory")?;
    for entry in entries {
        relative(&entry.relative)?;
    }
    let target = if intent.target.try_exists()? {
        Some(handles::open_directory(&intent.target, false)?)
    } else {
        None
    };
    let ours = target
        .as_ref()
        .map(|file| {
            Snapshot::read(file).map(|snapshot| {
                intent
                    .target_snapshot
                    .as_ref()
                    .is_some_and(|expected| matches(&snapshot, expected))
            })
        })
        .transpose()?
        .unwrap_or(false);
    if ours {
        let mut verified = Vec::new();
        if intent.temporary.is_some() {
            for entry in entries {
                cancelled(cancel)?;
                let path = intent.target.join(&entry.relative);
                let mut file = if entry.source.directory {
                    handles::open_directory(&path, false)?
                } else {
                    open_owned(&path, false)?
                };
                ensure!(
                    entry
                        .target
                        .as_ref()
                        .is_some_and(|expected| Snapshot::read(&file)
                            .is_ok_and(|actual| matches(&actual, expected))),
                    "Moved folder item changed; both copies preserved"
                );
                if !entry.source.directory {
                    ensure!(
                        Some(digest(&mut file, cancel)?.1) == entry.hash,
                        "Moved torrent checksum changed; both copies preserved"
                    );
                }
                verified.push(file);
            }
        }
        commit(store, &mut intent)?;
        if intent.temporary.is_some() && intent.source.try_exists()? {
            cleanup(
                &intent.source,
                &intent.source_snapshot.key,
                intent.tree.as_ref().unwrap(),
                false,
            )?;
        }
    } else {
        ensure!(
            !intent.committed,
            "Moved folder is missing or changed; recovery preserved all files"
        );
        let original = handles::open_directory(&intent.source, false)?;
        ensure!(
            Snapshot::read(&original)?.key == intent.source_snapshot.key,
            "Original folder changed; recovery preserved it"
        );
    }
    drop(target);
    if let Some((path, key)) = &intent.temporary
        && path.try_exists()?
    {
        cleanup(path, key, intent.tree.as_ref().unwrap(), true)?;
    }
    clear(store, &intent.job)
}

#[cfg(test)]
mod folder_tests {
    use super::*;
    fn run(store: &Store, job: &Job, copy: bool, cancel: &CancellationToken) -> Result<PathBuf> {
        let destination = store.paths.downloads.join("moved");
        std::fs::create_dir_all(&destination)?;
        let target = destination.join("Renamed pack");
        relocate(
            store,
            job,
            target.clone(),
            "Renamed pack".into(),
            destination,
            cancel,
            copy,
        )?;
        Ok(target)
    }
    #[test]
    fn folder_rename_and_verified_copy_keep_selected_files_extras_and_empty_directories() {
        for copy in [false, true] {
            let (_temp, store, job) = crate::torrent::completed_folder_fixture();
            let source = job.saved_path.as_ref().unwrap();
            std::fs::write(source.join("personal-note.bin"), b"keep me").unwrap();
            std::fs::create_dir(source.join("empty")).unwrap();
            let target = run(&store, &job, copy, &CancellationToken::new()).unwrap();
            assert!(!source.exists());
            assert_eq!(std::fs::read(target.join("a.bin")).unwrap(), b"harmless");
            assert_eq!(
                std::fs::read(target.join("nested/c.bin")).unwrap(),
                b"sample!!"
            );
            assert_eq!(
                std::fs::read(target.join("personal-note.bin")).unwrap(),
                b"keep me"
            );
            assert!(target.join("empty").is_dir());
            assert!(!target.join("skipped.bin").exists());
            let saved = store.get(&job.id).unwrap().unwrap();
            assert_eq!(saved.saved_path, Some(target.clone()));
            assert_eq!(saved.name, "Renamed pack");
            assert_eq!(
                crate::torrent::completed_files(&store, &saved).unwrap()[1].0,
                target.join("nested/c.bin")
            );
            assert!(!is_pending(&store, &job.id).unwrap());
        }
    }
    #[test]
    fn folder_collision_and_cancellation_preserve_the_original() {
        let (_temp, store, job) = crate::torrent::completed_folder_fixture();
        let target = store.paths.downloads.join("moved/Renamed pack");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("mine.bin"), b"mine").unwrap();
        assert!(run(&store, &job, false, &CancellationToken::new()).is_err());
        assert_eq!(std::fs::read(target.join("mine.bin")).unwrap(), b"mine");
        assert!(job.saved_path.as_ref().unwrap().join("a.bin").exists());
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(
            relocate(
                &store,
                &job,
                store.paths.downloads.join("other"),
                "other".into(),
                store.paths.downloads.clone(),
                &cancel,
                true
            )
            .is_err()
        );
        assert!(!is_pending(&store, &job.id).unwrap());
    }
    #[test]
    fn folder_copy_recovery_commits_only_verified_files_after_publication() {
        for copy in [false, true] {
            let (_temp, store, job) = crate::torrent::completed_folder_fixture();
            let _gate = tests::Gate::new(&job.id, copy, 2);
            let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run(&store, &job, copy, &CancellationToken::new())
            }));
            assert!(crashed.is_err());
            assert!(is_pending(&store, &job.id).unwrap());
            super::super::recover(&store, &job.id, &CancellationToken::new()).unwrap();
            let saved = store.get(&job.id).unwrap().unwrap();
            assert_eq!(
                std::fs::read(saved.saved_path.unwrap().join("nested/c.bin")).unwrap(),
                b"sample!!"
            );
            assert!(!job.saved_path.unwrap().exists());
            assert!(!is_pending(&store, &job.id).unwrap());
        }
    }
    #[cfg(windows)]
    #[test]
    fn folder_copy_keeps_mark_of_the_web_and_other_streams() {
        let (_temp, store, job) = crate::torrent::completed_folder_fixture();
        let path = job.saved_path.as_ref().unwrap().join("a.bin");
        std::fs::write(
            format!("{}:Zone.Identifier", path.display()),
            b"[ZoneTransfer]\r\nZoneId=3\r\n",
        )
        .unwrap();
        let target = run(&store, &job, true, &CancellationToken::new()).unwrap();
        assert_eq!(
            std::fs::read(format!(
                "{}:Zone.Identifier",
                target.join("a.bin").display()
            ))
            .unwrap(),
            b"[ZoneTransfer]\r\nZoneId=3\r\n"
        );
    }
}
