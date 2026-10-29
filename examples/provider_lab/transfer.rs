use super::{Grants, MAX_FILE, Plan, crypto::MAX_CHUNK, runtime::Runtime, sha256};
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    time::Duration,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct Options {
    pub chunk_bytes: usize,
    pub pause_after_chunks: Option<usize>,
    pub callback_budget: Duration,
}
impl Default for Options {
    fn default() -> Self {
        // Deliberately unaligned to exercise byte offsets across AES blocks.
        Self {
            chunk_bytes: 65_539,
            pause_after_chunks: None,
            callback_budget: Duration::from_secs(2),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Paused,
    Complete,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub state: State,
    pub bytes: u64,
    pub resumed_from: u64,
    pub transformed_chunks: usize,
    pub output_sha256: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    schema: u8,
    script_sha256: String,
    binding: String,
    plan: Plan,
    offset: u64,
    prefix_sha256: String,
}

// Only Rust receives paths. On Windows, refuse reparse points and hard links,
// and retain handles denying writers/deletion while their content is in use.
fn open(path: &Path, write: bool, create: bool, published: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(write).create(create);
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::*;
    options
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .share_mode(FILE_SHARE_READ);
    let file = options.open(path)?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // The file owns the borrowed handle for the entire call.
    ensure!(
        unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } != 0,
        "Cannot inspect lab file"
    );
    ensure!(
        file.metadata()?.is_file() && info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT == 0,
        "Lab path is not a regular unlinked file"
    );
    ensure!(
        info.nNumberOfLinks == 1 || (published && info.nNumberOfLinks == 2),
        "Unexpected lab hard link"
    );
    Ok(file)
}

fn hash_prefix(file: &mut File, size: u64, cancel: &CancellationToken) -> Result<Sha256> {
    ensure!(size <= MAX_FILE, "Lab file exceeds limit");
    file.seek(SeekFrom::Start(0))?;
    let mut left = size;
    let mut buffer = vec![0; MAX_CHUNK];
    let mut hash = Sha256::new();
    while left > 0 {
        ensure!(!cancel.is_cancelled(), "Provider cancelled");
        let n = buffer.len().min(left as usize);
        file.read_exact(&mut buffer[..n])?;
        hash.update(&buffer[..n]);
        left -= n as u64;
    }
    Ok(hash)
}

fn save_checkpoint(root: &Path, checkpoint: &Checkpoint) -> Result<()> {
    let pending = root.join(format!("checkpoint-{}.pending", uuid::Uuid::new_v4()));
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)?;
    output.write_all(&serde_json::to_vec(checkpoint)?)?;
    output.sync_all()?;
    drop(output);
    // Atomic replacement on the same volume. A crashed pending file is ignored
    // and ultimately removed by the owner of the private TempDir.
    fs::rename(pending, root.join("checkpoint.json"))?;
    Ok(())
}

fn binding(source: &str, plan: &Plan, grants: &Grants, chunk: usize) -> Result<String> {
    let mut hash = Sha256::new();
    hash.update(serde_json::to_vec(&(
        "provider-lab-0",
        sha256(source.as_bytes()),
        plan.identity(),
        chunk,
    ))?);
    let key = grants
        .keys
        .get(&plan.key_id)
        .context("Key is not granted")?;
    // Bind the key without persisting its bytes. This lab accepts raw AES keys,
    // not passwords. A production vault should use a versioned key reference.
    hash.update(key);
    Ok(format!("{:x}", hash.finalize()))
}

pub fn run(
    source: &str,
    grants: &Grants,
    root: &Path,
    options: &Options,
    cancel: CancellationToken,
) -> Result<Report> {
    ensure!(
        (1..=MAX_CHUNK).contains(&options.chunk_bytes),
        "Invalid chunk size"
    );
    ensure!(!cancel.is_cancelled(), "Provider cancelled");
    let meta = fs::symlink_metadata(root)?;
    ensure!(
        meta.is_dir() && !meta.file_type().is_symlink(),
        "Use a private lab directory"
    );
    let lock = open(&root.join("worker.lock"), true, true, false)?;
    lock.try_lock_exclusive()
        .context("Another worker owns this lab run")?;
    let checkpoint_path = root.join("checkpoint.json");
    let previous: Option<Checkpoint> = if checkpoint_path.try_exists()? {
        let file = open(&checkpoint_path, false, false, false)?;
        ensure!(file.metadata()?.len() <= 16_384, "Oversized checkpoint");
        let cp: Checkpoint = serde_json::from_reader(file)?;
        ensure!(
            cp.schema == 1 && cp.script_sha256 == sha256(source.as_bytes()),
            "Script changed or checkpoint version is unsupported; start a fresh run"
        );
        cp.plan.validate()?;
        ensure!(cp.offset <= cp.plan.size, "Invalid checkpoint offset");
        Some(cp)
    } else {
        None
    };
    let runtime = Runtime::new(source, grants, cancel.clone(), options.callback_budget)?;
    let plan = runtime.resolve(previous.as_ref().map(|cp| &cp.plan))?;
    ensure!(
        plan.identity() == grants.metadata.identity(),
        "Plan differs from the granted content"
    );
    let fingerprint = binding(source, &plan, grants, options.chunk_bytes)?;
    if let Some(cp) = &previous {
        ensure!(
            cp.binding == fingerprint && cp.plan.identity() == plan.identity(),
            "Content, key or processing settings changed; start a fresh run"
        );
        ensure!(
            cp.offset == plan.size || cp.offset % options.chunk_bytes as u64 == 0,
            "Checkpoint is not at a completed chunk boundary"
        );
    }
    let input_path = grants
        .sources
        .get(&plan.source)
        .context("Source is not granted")?;
    let mut input = open(input_path, false, false, false)?;
    ensure!(input.metadata()?.len() == plan.size, "Source size changed");
    ensure!(
        format!(
            "{:x}",
            hash_prefix(&mut input, plan.size, &cancel)?.finalize()
        ) == plan.input_sha256,
        "Encrypted source hash mismatch"
    );
    let mut cp = previous.unwrap_or(Checkpoint {
        schema: 1,
        script_sha256: sha256(source.as_bytes()),
        binding: fingerprint,
        plan: plan.clone(),
        offset: 0,
        prefix_sha256: sha256(&[]),
    });
    let resumed_from = cp.offset;
    let partial = root.join("output.partial");
    let final_path = root.join("output.bin");
    let published = final_path.try_exists()?;
    if !checkpoint_path.try_exists()? {
        ensure!(
            !partial.try_exists()? && !published,
            "Unowned output already exists"
        );
        save_checkpoint(root, &cp)?;
    }
    ensure!(
        cp.offset == 0 || partial.try_exists()?,
        "Checkpoint output is missing"
    );
    let mut output = open(
        &partial,
        !published,
        cp.offset == 0 && !published,
        published,
    )?;
    ensure!(
        output.metadata()?.len() >= cp.offset,
        "Checkpoint output is truncated"
    );
    let mut hash = hash_prefix(&mut output, cp.offset, &cancel)?;
    ensure!(
        format!("{:x}", hash.clone().finalize()) == cp.prefix_sha256,
        "Checkpoint output hash mismatch"
    );
    if published {
        let final_file = open(&final_path, false, false, true)?;
        ensure!(
            same_file::Handle::from_file(output.try_clone()?)?
                == same_file::Handle::from_file(final_file)?
                && cp.offset == plan.size
                && output.metadata()?.len() == plan.size
                && cp.prefix_sha256 == plan.output_sha256,
            "Existing output is not this completed run"
        );
        return Ok(Report {
            state: State::Complete,
            bytes: cp.offset,
            resumed_from,
            transformed_chunks: 0,
            output_sha256: Some(cp.prefix_sha256),
        });
    }
    // Discard only a private, uncommitted tail after checking the saved prefix.
    // Ciphertext is never changed, so a crash cannot cause double decryption.
    output.set_len(cp.offset)?;
    output.seek(SeekFrom::Start(cp.offset))?;
    input.seek(SeekFrom::Start(cp.offset))?;
    let mut chunks = 0;
    let pause = |offset, chunks| Report {
        state: State::Paused,
        bytes: offset,
        resumed_from,
        transformed_chunks: chunks,
        output_sha256: None,
    };
    while cp.offset < plan.size {
        if cancel.is_cancelled() || options.pause_after_chunks.is_some_and(|n| chunks >= n) {
            return Ok(pause(cp.offset, chunks));
        }
        let len = (plan.size - cp.offset).min(options.chunk_bytes as u64) as usize;
        let mut bytes = vec![0; len];
        input.read_exact(&mut bytes)?;
        let transformed = match runtime.transform(&bytes, cp.offset, &plan) {
            Ok(bytes) => bytes,
            Err(_) if cancel.is_cancelled() => return Ok(pause(cp.offset, chunks)),
            Err(error) => return Err(error),
        };
        if cancel.is_cancelled() {
            return Ok(pause(cp.offset, chunks));
        }
        output.write_all(&transformed)?;
        output.sync_all()?;
        hash.update(&transformed);
        cp.offset += len as u64;
        cp.prefix_sha256 = format!("{:x}", hash.clone().finalize());
        cp.plan = plan.clone();
        save_checkpoint(root, &cp)?;
        chunks += 1;
    }
    ensure!(
        cp.prefix_sha256 == plan.output_sha256,
        "Plaintext hash mismatch; output was not published"
    );
    if let Err(error) = runtime.finish(&plan, &cp.prefix_sha256, cp.offset) {
        if cancel.is_cancelled() {
            return Ok(pause(cp.offset, chunks));
        }
        return Err(error);
    }
    if cancel.is_cancelled() {
        return Ok(pause(cp.offset, chunks));
    }
    output.sync_all()?;
    // Creating a new hard link cannot overwrite an unrelated existing file.
    // Keep output.partial as the recovery anchor, including across a crash here.
    fs::hard_link(&partial, &final_path)
        .context("Cannot publish verified output without overwriting")?;
    Ok(Report {
        state: State::Complete,
        bytes: cp.offset,
        resumed_from,
        transformed_chunks: chunks,
        output_sha256: Some(cp.prefix_sha256),
    })
}
