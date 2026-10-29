//! Core-owned transform checkpoints. A partial output is never treated as a finished file.
use super::*;
use crate::{download_file, transfer_activity::Phase};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    version: u32,
    binding: String,
    offset: u64,
    prefix_sha256: String,
}

fn hash_prefix(file: &mut File, length: u64, cancel: &CancellationToken) -> Result<Sha256> {
    file.seek(SeekFrom::Start(0))?;
    let mut hash = Sha256::new();
    let mut left = length;
    let mut bytes = vec![0; CHUNK_BYTES];
    while left > 0 {
        ensure!(!cancel.is_cancelled(), "Provider cancelled");
        let count = left.min(bytes.len() as u64) as usize;
        file.read_exact(&mut bytes[..count])?;
        hash.update(&bytes[..count]);
        left -= count as u64;
    }
    Ok(hash)
}
fn checkpoint(root: &Path, state: &Checkpoint) -> Result<()> {
    let pending = root.join("provider-progress.pending");
    let mut file = download_file::fresh_sync(&pending, false)?;
    file.write_all(&serde_json::to_vec(state)?)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(pending, root.join("provider-progress.json"))?;
    Ok(())
}

pub(crate) async fn transform(
    store: &Store,
    job: &Job,
    input: File,
    cancel: &CancellationToken,
) -> Result<PathBuf> {
    store.activity.set(&job.id, Phase::Processing);
    let store = store.clone();
    let job = job.clone();
    let cancel = cancel.clone();
    tokio::task::spawn_blocking(move || process(&store, &job, input, &cancel)).await?
}

fn process(
    store: &Store,
    job: &Job,
    mut input: File,
    cancel: &CancellationToken,
) -> Result<PathBuf> {
    let state = job.provider.as_ref().context("Missing provider")?;
    let plan = state.plan.as_ref().context("Missing provider plan")?;
    ensure!(
        input.metadata()?.len() == plan.size,
        "Provider encrypted source size changed"
    );
    let root = store.paths.parts(&job.id)?;
    let _parent = download_file::guard_download_parent(&root.join("provider-output.part"))?;
    let path = root.join("provider-output.part");
    let progress = root.join("provider-progress.json");
    let binding = state.binding()?;
    let (mut output, mut saved, mut hash) = if progress.try_exists()? {
        let mut bytes = Vec::new();
        download_file::read_part_sync(&progress)?
            .take(16_385)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= 16_384,
            "Provider checkpoint exceeds its size limit"
        );
        let saved: Checkpoint = serde_json::from_slice(&bytes)
            .context("Invalid provider checkpoint. Restart this download.")?;
        ensure!(
            saved.version == VERSION && saved.binding == binding && saved.offset <= plan.size,
            "Provider checkpoint does not match this script and file. Restart this download."
        );
        // Missing output never becomes a new empty file when a checkpoint exists.
        let mut output = download_file::read_write_part_sync(&path)?;
        ensure!(
            output.metadata()?.len() >= saved.offset,
            "Saved provider output is incomplete. Restart this download."
        );
        let hash = hash_prefix(&mut output, saved.offset, cancel)?;
        ensure!(
            format!("{:x}", hash.clone().finalize()) == saved.prefix_sha256,
            "Saved provider output changed. Restart this download."
        );
        output.set_len(saved.offset)?; // Only uncommitted private bytes may be discarded.
        output.seek(SeekFrom::Start(saved.offset))?;
        (output, saved, hash)
    } else {
        ensure!(
            !path.try_exists()?,
            "Provider output has no checkpoint. Restart this download."
        );
        let output = download_file::fresh_sync(&path, true)?;
        let hash = Sha256::new();
        let saved = Checkpoint {
            version: VERSION,
            binding,
            offset: 0,
            prefix_sha256: format!("{:x}", hash.clone().finalize()),
        };
        checkpoint(&root, &saved)?;
        (output, saved, hash)
    };
    let refreshed = {
        let mut resolver = super::process::Worker::start(
            &store.paths,
            &state.script,
            super::process::Role::Resolver,
            cancel,
        )?;
        resolver.resolve(&job.url, Some(plan), cancel)?.content()
    }; // Kill and join the network-capable worker before supplying any file bytes.
    ensure!(
        plan == &refreshed,
        "Provider content changed before processing. Restart this download."
    );
    let mut worker = super::process::Worker::start(
        &store.paths,
        &state.script,
        super::process::Role::Processor,
        cancel,
    )?;
    input.seek(SeekFrom::Start(saved.offset))?;
    let mut bytes = vec![0; CHUNK_BYTES];
    while saved.offset < plan.size {
        ensure!(!cancel.is_cancelled(), "Provider cancelled");
        let count = (plan.size - saved.offset).min(CHUNK_BYTES as u64) as usize;
        input.read_exact(&mut bytes[..count])?;
        let transformed = worker.transform(&refreshed, &bytes[..count], saved.offset, cancel)?;
        ensure!(!cancel.is_cancelled(), "Provider cancelled");
        output.write_all(&transformed)?;
        output.sync_all()?;
        hash.update(&transformed);
        saved.offset += count as u64;
        saved.prefix_sha256 = format!("{:x}", hash.clone().finalize());
        checkpoint(&root, &saved)?;
    }
    ensure!(
        saved.prefix_sha256 == plan.output_sha256,
        "Provider plaintext checksum does not match. The file was not published."
    );
    worker.finish(&refreshed, cancel)?;
    ensure!(!cancel.is_cancelled(), "Provider cancelled");
    drop(output);
    Ok(path)
}
