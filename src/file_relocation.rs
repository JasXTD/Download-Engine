//! Completed-file relocation: an additive journal bridges filesystem and SQLite commits.
use crate::{
    download_file,
    model::{Job, Status},
    store::Store,
};
use anyhow::{Context, Result, ensure};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;

mod handles;
use handles::{FileKey, Snapshot, delete_owned, open_owned, rename_owned};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Intent {
    job: String,
    source: PathBuf,
    target: PathBuf,
    source_snapshot: Snapshot,
    // A precreated staging file is identified before any payload is copied.
    temporary: Option<(PathBuf, FileKey)>,
    target_snapshot: Option<Snapshot>,
    target_hash: Option<[u8; 32]>,
    name: String,
    directory: PathBuf,
    committed: bool,
}

pub(crate) fn initialize(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS file_relocations (
        job_id TEXT PRIMARY KEY REFERENCES jobs(id) ON DELETE RESTRICT,
        data TEXT NOT NULL);",
    )?;
    Ok(())
}

pub(crate) fn pending(store: &Store) -> Result<Vec<String>> {
    let conn = store.connect()?;
    let mut statement = conn.prepare("SELECT job_id FROM file_relocations ORDER BY rowid")?;
    Ok(statement
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

pub(crate) fn is_pending(store: &Store, job: &str) -> Result<bool> {
    Ok(store.connect()?.query_row(
        "SELECT EXISTS(SELECT 1 FROM file_relocations WHERE job_id=?1)",
        [job],
        |r| r.get(0),
    )?)
}

fn save(store: &Store, intent: &Intent) -> Result<()> {
    let count = store.connect()?.execute(
        "UPDATE file_relocations SET data=?1 WHERE job_id=?2",
        params![serde_json::to_string(intent)?, intent.job],
    )?;
    ensure!(count == 1, "File operation record is missing");
    Ok(())
}
fn clear(store: &Store, id: &str) -> Result<()> {
    store
        .connect()?
        .execute("DELETE FROM file_relocations WHERE job_id=?1", [id])?;
    Ok(())
}

fn commit(store: &Store, intent: &mut Intent) -> Result<()> {
    let mut conn = store.connect()?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let data: String = tx.query_row("SELECT data FROM jobs WHERE id=?1", [&intent.job], |r| {
        r.get(0)
    })?;
    let mut job = crate::store::decode_job(&intent.job, &data)?;
    ensure!(
        job.status == Status::Complete,
        "Download state changed during the file operation"
    );
    ensure!(
        job.saved_path.as_ref() == Some(&intent.source)
            || job.saved_path.as_ref() == Some(&intent.target),
        "Saved file changed during the file operation"
    );
    // Merge only the relocation fields, preserving all other download data.
    job.saved_path = Some(intent.target.clone());
    job.name = intent.name.clone();
    job.directory = intent.directory.clone();
    job.validate_persisted()?;
    let mut data: serde_json::Value = serde_json::from_str(&data)?;
    data["saved_path"] = serde_json::to_value(&job.saved_path)?;
    data["name"] = serde_json::to_value(&job.name)?;
    data["directory"] = serde_json::to_value(&job.directory)?;
    intent.committed = true;
    tx.execute(
        "UPDATE jobs SET data=?1 WHERE id=?2",
        params![serde_json::to_string(&data)?, job.id],
    )?;
    tx.execute(
        "UPDATE file_relocations SET data=?1 WHERE job_id=?2",
        params![serde_json::to_string(intent)?, intent.job],
    )?;
    tx.commit()?;
    Ok(())
}

fn cancelled(cancel: &CancellationToken) -> Result<()> {
    ensure!(
        !cancel.is_cancelled(),
        "File operation cancelled; the original file was preserved"
    );
    Ok(())
}

fn copy_stream(source: &mut File, target: &mut File, cancel: &CancellationToken) -> Result<()> {
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        cancelled(cancel)?;
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        target.write_all(&buffer[..count])?;
        #[cfg(test)]
        tests::after_copy_chunk(cancel);
    }
    target.sync_all()?;
    Ok(())
}

fn digest(file: &mut File, cancel: &CancellationToken) -> Result<(u64, [u8; 32])> {
    use sha2::{Digest, Sha256};
    file.seek(SeekFrom::Start(0))?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 256 * 1024];
    let mut size = 0;
    loop {
        cancelled(cancel)?;
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        size += count as u64;
        hash.update(&buffer[..count]);
    }
    Ok((size, hash.finalize().into()))
}

pub(crate) fn relocate(
    store: &Store,
    job: &Job,
    target: PathBuf,
    name: String,
    directory: PathBuf,
    cancel: &CancellationToken,
) -> Result<()> {
    relocate_inner(store, job, target, name, directory, cancel, false)
}

fn relocate_inner(
    store: &Store,
    job: &Job,
    target: PathBuf,
    name: String,
    directory: PathBuf,
    cancel: &CancellationToken,
    force_copy: bool,
) -> Result<()> {
    #[cfg(test)]
    let _hook = tests::enter(&job.id);
    #[cfg(test)]
    let force_copy = force_copy || tests::force_copy();
    ensure!(
        job.status == Status::Complete,
        "Only finished downloads can be moved or renamed"
    );
    ensure!(
        !is_pending(store, &job.id)?,
        "A file operation needs recovery before changing this download"
    );
    let source = job
        .saved_path
        .clone()
        .context("This download has no saved file")?;
    ensure!(target != source, "The file is already at this location");
    let _source_parents = download_file::guard_parents(&source)?;
    let _target_parents = download_file::guard_parents(&target)?;
    ensure!(
        !target.try_exists()? || handles::same_name_ignoring_case(&source, &target),
        "{} already exists. Choose another name or folder.",
        target.display()
    );
    cancelled(cancel)?;
    let mut original = open_owned(&source, false)?;
    let source_snapshot = Snapshot::read(&original)?;
    let copied = force_copy
        || !download_file::same_volume(
            &original,
            _target_parents.last().context("Missing target parent")?,
        )?;
    if copied {
        handles::check_copy(&original)?;
    }
    let mut staging = if copied {
        let path = target.with_file_name(format!(".de-relocate-{}.moving", uuid::Uuid::new_v4()));
        let file = open_owned(&path, true)?;
        Some((path, file))
    } else {
        None
    };
    let mut intent = Intent {
        job: job.id.clone(),
        source,
        target,
        source_snapshot: source_snapshot.clone(),
        temporary: staging
            .as_ref()
            .map(|(p, f)| -> Result<_> { Ok((p.clone(), Snapshot::read(f)?.key)) })
            .transpose()?,
        target_snapshot: (!copied).then_some(source_snapshot),
        target_hash: None,
        name,
        directory,
        committed: false,
    };
    let inserted = store.connect().and_then(|conn| {
        conn.execute(
            "INSERT INTO file_relocations(job_id,data) VALUES(?1,?2)",
            params![intent.job, serde_json::to_string(&intent)?],
        )?;
        Ok(())
    });
    if let Err(error) = inserted {
        if let Some((_, file)) = &staging {
            delete_owned(file)?;
        }
        return Err(error);
    }
    let mut published = false;
    let result = (|| -> Result<()> {
        if let Some((path, file)) = &mut staging {
            copy_stream(&mut original, file, cancel)?;
            // Preserve named streams, including Mark of the Web, before publication.
            handles::copy_streams(&intent.source, path, cancel, copy_stream)?;
            #[cfg(test)]
            tests::after_copy(file)?;
            let source_digest = digest(&mut original, cancel)?;
            ensure!(
                source_digest == digest(file, cancel)?,
                "The copied file failed checksum verification; original preserved"
            );
            intent.target_hash = Some(source_digest.1);
            handles::copy_metadata(&original, file)?;
            intent.target_snapshot = Some(Snapshot::read(file)?);
            save(store, &intent)?;
            cancelled(cancel)?;
            #[cfg(test)]
            tests::before_publish();
            rename_owned(file, path, &intent.target)?;
        } else {
            cancelled(cancel)?;
            rename_owned(&original, &intent.source, &intent.target)?;
        }
        published = true;
        #[cfg(test)]
        tests::after_publish(&intent.job);
        // Cancellation after publication must finish the metadata commit. The
        // durable intent remains if this commit fails; never guess at rollback.
        commit(store, &mut intent)
            .context("File published; its record will be recovered when the app restarts")?;
        #[cfg(test)]
        tests::after_commit();
        if copied {
            delete_owned(&original).context("File moved, but the original still needs cleanup")?;
        }
        drop(staging.take());
        clear(store, &intent.job)?;
        Ok(())
    })();
    if result.is_err() && !published {
        // The source is still guarded and unchanged. Only our exact staging
        // handle can be removed; never delete a replacement by pathname.
        if let Some((_, file)) = &staging {
            delete_owned(file)?;
        }
        drop(staging.take());
        clear(store, &intent.job)?;
    }
    result
}

pub(crate) fn recover(store: &Store, id: &str, cancel: &CancellationToken) -> Result<()> {
    cancelled(cancel)?;
    let conn = store.connect()?;
    let data: Option<String> = conn
        .query_row(
            "SELECT data FROM file_relocations WHERE job_id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(data) = data else {
        return Ok(());
    };
    drop(conn);
    let mut intent: Intent = serde_json::from_str(&data)?;
    ensure!(
        intent.job == id,
        "File operation identifier does not match its row"
    );
    crate::model::validate_job_id(id)?;
    let _source_parents = download_file::guard_parents(&intent.source)?;
    let _target_parents = download_file::guard_parents(&intent.target)?;
    let mut target = if intent.target.try_exists()? {
        Some(open_owned(&intent.target, false)?)
    } else {
        None
    };
    let target_matches = target
        .as_ref()
        .map(|file| -> Result<bool> {
            Ok(intent.target_snapshot.as_ref() == Some(&Snapshot::read(file)?))
        })
        .transpose()?
        .unwrap_or(false);
    if target_matches {
        if let Some(hash) = intent.target_hash {
            ensure!(
                digest(target.as_mut().context("Missing moved file")?, cancel)?.1 == hash,
                "Moved file checksum changed; both files were preserved for review"
            );
        }
        cancelled(cancel)?;
        commit(store, &mut intent)?;
        if intent.temporary.is_some() && intent.source.try_exists()? {
            let original = open_owned(&intent.source, false)?;
            ensure!(
                Snapshot::read(&original)? == intent.source_snapshot,
                "Original file changed after the move; both files were preserved"
            );
            delete_owned(&original)?;
        }
    } else {
        ensure!(
            !intent.committed,
            "Moved file changed or is missing; recovery preserved all files for review"
        );
        let original = open_owned(&intent.source, false)
            .context("Cannot identify either side of an interrupted move; files were preserved")?;
        ensure!(
            Snapshot::read(&original)? == intent.source_snapshot,
            "Original file changed; recovery preserved all files for review"
        );
        // A different target belongs to someone else. Leave it alone.
    }
    if let Some((path, key)) = &intent.temporary
        && path.try_exists()?
    {
        let _parents = download_file::guard_parents(path)?;
        let temporary = open_owned(path, false)?;
        ensure!(
            &Snapshot::read(&temporary)?.key == key,
            "Staging file changed; it was preserved for review"
        );
        delete_owned(&temporary)?;
    }
    clear(store, id)
}

#[cfg(test)]
pub(crate) mod tests;
