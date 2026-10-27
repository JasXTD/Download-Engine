//! Avoid copying a complete single part; retain file ownership through publication.
use super::*;
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read};
use tokio::io::AsyncReadExt;

struct Prepared {
    path: PathBuf,
    file: File,
    hash: String,
    copied: bool,
    _parents: Vec<File>,
}

pub(super) async fn finish(
    store: &Store,
    job: &mut Job,
    cancel: &CancellationToken,
    sources: Vec<PathBuf>,
) -> Result<Option<FinishedFile>> {
    store.activity.set(&job.id, Phase::Assembling);
    store.save(job)?;
    let direct = if sources.len() == 1 && job.hls.is_none() {
        let snapshot = job.clone();
        let token = cancel.clone();
        let activity = store.activity.clone();
        let source = sources[0].clone();
        // Await the worker after cancellation: no hash/file guard may outlive the job.
        tokio::task::spawn_blocking(move || prepare_direct(&snapshot, source, &token, &activity))
            .await??
    } else {
        Direct::CopyRequired
    };
    let prepared = match direct {
        Direct::Ready(prepared) => prepared,
        Direct::Cancelled => return Ok(None),
        Direct::CopyRequired => {
            let Some(prepared) = prepare_copy(job, sources, cancel).await? else {
                return Ok(None);
            };
            prepared
        }
    };
    if cancel.is_cancelled() {
        discard_cancelled(prepared)?;
        return Ok(None);
    }
    job.integrity.actual = Some(prepared.hash);
    job.integrity.check()?;
    if let Some(file) = synchronization::reuse_unchanged(job, cancel).await? {
        return Ok(Some(file));
    }
    if cancel.is_cancelled() {
        if prepared.copied {
            drop(prepared.file);
            std::fs::remove_file(&prepared.path)?;
        }
        return Ok(None);
    }
    // The source handle denies other writers/deletion on Windows while the
    // existing intent-before-hard-link protocol publishes these verified bytes.
    let path = publish_without_overwrite(store, job, &prepared.path).await?;
    Ok(Some(FinishedFile {
        path,
        guard: Some(prepared.file),
    }))
}

fn discard_cancelled(prepared: Prepared) -> Result<()> {
    drop(prepared.file);
    if prepared.copied {
        std::fs::remove_file(prepared.path)?;
    }
    Ok(())
}

enum Direct {
    Ready(Prepared),
    CopyRequired,
    Cancelled,
}

fn prepare_direct(
    job: &Job,
    source: PathBuf,
    cancel: &CancellationToken,
    activity: &crate::transfer_activity::Activity,
) -> Result<Direct> {
    let staging = job.directory.join(format!(".idm-ex-{}.assembling", job.id));
    let parents = vec![
        download_file::guard_download_parent(&staging)?,
        download_file::guard_download_parent(&source)?,
    ];
    let mut file = download_file::read_part_sync(&source)?;
    if !download_file::same_volume(&file, &parents[0])? {
        return Ok(Direct::CopyRequired);
    }
    activity.set(&job.id, Phase::Verifying);
    // Also mark parts written by older builds before they become the final file.
    download_file::mark_internet(&source)?;
    let mut buffer = vec![0; 256 * 1024];
    let mut hash = Sha256::new();
    let mut total = 0u64;
    loop {
        if cancel.is_cancelled() {
            return Ok(Direct::Cancelled);
        }
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        total = total
            .checked_add(n as u64)
            .context("Assembled size overflow")?;
        ensure!(total <= job.downloaded, "Assembled size mismatch");
        hash.update(&buffer[..n]);
        #[cfg(test)]
        super::finalization_tests::after_chunk(&source);
    }
    ensure!(total == job.downloaded, "Assembled size mismatch");
    if cancel.is_cancelled() {
        return Ok(Direct::Cancelled);
    }
    Ok(Direct::Ready(Prepared {
        path: source,
        file,
        hash: format!("{:x}", hash.finalize()),
        copied: false,
        _parents: parents,
    }))
}

async fn prepare_copy(
    job: &Job,
    sources: Vec<PathBuf>,
    cancel: &CancellationToken,
) -> Result<Option<Prepared>> {
    ensure!(!sources.is_empty(), "Missing saved download parts");
    let staging = job.directory.join(format!(".idm-ex-{}.assembling", job.id));
    let (destination, source) = (staging.clone(), sources[0].clone());
    let parents = tokio::task::spawn_blocking(move || -> Result<Vec<File>> {
        Ok(vec![
            download_file::guard_download_parent(&destination)?,
            download_file::guard_download_parent(&source)?,
        ])
    })
    .await??;
    let mut output = download_file::fresh(&staging).await?;
    // Keep the original asynchronous read/write overlap for multi-part and
    // cross-volume copies. A synchronous-copy experiment did not improve it.
    let outcome: Result<Option<String>> = async {
        let mut buffer = vec![0; 256 * 1024];
        let mut hash = Sha256::new();
        let mut total = 0u64;
        for (index, path) in sources.iter().enumerate() {
            let mut input = download_file::read_part(path).await?;
            let mut part_hash = Sha256::new();
            let mut part_size = 0u64;
            loop {
                if cancel.is_cancelled() {
                    return Ok(None);
                }
                let n = input.read(&mut buffer).await?;
                if n == 0 {
                    break;
                }
                total = total
                    .checked_add(n as u64)
                    .context("Assembled size overflow")?;
                ensure!(total <= job.downloaded, "Assembled size mismatch");
                #[cfg(test)]
                storage_tests::before_write(&staging, n)?;
                output.write_all(&buffer[..n]).await?;
                hash.update(&buffer[..n]);
                if job.hls.is_some() {
                    part_hash.update(&buffer[..n]);
                    part_size += n as u64;
                }
                #[cfg(test)]
                super::finalization_tests::after_chunk(&sources[0]);
            }
            if let Some(hls) = &job.hls {
                let saved = hls
                    .completed
                    .get(index)
                    .context("Incomplete HLS download")?;
                ensure!(
                    part_size == saved.size
                        && format!("{:x}", part_hash.finalize()) == saved.sha256,
                    "Saved HLS segment changed before assembly. Restart this download."
                );
            }
        }
        ensure!(total == job.downloaded, "Assembled size mismatch");
        Ok(Some(format!("{:x}", hash.finalize())))
    }
    .await;
    // A Tokio write may still be in flight after write_all returns. Join it on
    // every outcome before releasing the handle or deleting cancelled staging.
    output.flush().await?;
    let Some(hash) = outcome? else {
        drop(output);
        tokio::fs::remove_file(&staging).await?;
        return Ok(None);
    };
    output.sync_all().await?;
    ensure!(
        output.metadata().await?.len() == job.downloaded,
        "Assembled size mismatch"
    );
    Ok(Some(Prepared {
        path: staging,
        file: output.into_std().await,
        hash,
        copied: true,
        _parents: parents,
    }))
}

/// A single-part publication uses the original cache file as its recovery
/// anchor. No new saved-job format is needed; multi-part/media jobs keep staging.
fn direct_source(job: &Job, root: &Path) -> Option<PathBuf> {
    if job.hls.is_some() {
        None
    } else if job.ftp.is_some() {
        Some(root.join("ftp.part"))
    } else if let Some(plan) = &job.plan {
        (plan.segments.len() == 1).then(|| part_path(root, 0))
    } else {
        Some(root.join("whole.part"))
    }
}

pub(super) async fn recover(
    store: &Store,
    job: &mut Job,
    root: &Path,
    cancel: &CancellationToken,
) -> Result<Option<FinishedFile>> {
    let Some(path) = job.saved_path.clone() else {
        return Ok(None);
    };
    let staging = job.directory.join(format!(".idm-ex-{}.assembling", job.id));
    for anchor in std::iter::once(staging).chain(direct_source(job, root)) {
        // A name collision can leave an intent pointing at somebody else's
        // file. Do not request exclusive read guards on that unrelated file.
        if !anchor.try_exists()? || !path.try_exists()? || !same_file::is_same_file(&anchor, &path)?
        {
            continue;
        }
        let published = path.clone();
        let token = cancel.clone();
        let total = job.total;
        let downloaded = job.downloaded;
        store.activity.set(&job.id, Phase::Verifying);
        let recovered = tokio::task::spawn_blocking(move || -> Result<Option<(String, File)>> {
            let _source_parent = download_file::guard_download_parent(&anchor)?;
            let _destination_parent = download_file::guard_download_parent(&published)?;
            let source = download_file::read_completed(&anchor)?;
            let destination = download_file::read_completed(&published)?;
            if same_file::Handle::from_file(source.try_clone()?)?
                != same_file::Handle::from_file(destination.try_clone()?)?
            {
                return Ok(None);
            }
            let size = source.metadata()?.len();
            ensure!(
                Some(size) == total && size == downloaded,
                "Published file size changed"
            );
            let (actual, guard) = crate::integrity::hash_file(&anchor, &token)?;
            download_file::mark_internet(&anchor)?;
            Ok(Some((actual, guard)))
        })
        .await??;
        if let Some((actual, guard)) = recovered {
            if let Some(previous) = &job.integrity.actual {
                ensure!(
                    previous.eq_ignore_ascii_case(&actual),
                    "Published file checksum changed"
                );
            }
            job.integrity.actual = Some(actual);
            job.integrity.check()?;
            return Ok(Some(FinishedFile {
                path,
                guard: Some(guard),
            }));
        }
    }
    Ok(None)
}
