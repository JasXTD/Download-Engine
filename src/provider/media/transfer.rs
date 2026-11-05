use super::*;
use crate::{
    bandwidth::Limiter,
    download_file, engine,
    model::Job,
    provider::{
        CHUNK_BYTES,
        process::{Role, Worker},
    },
    store::Store,
    synchronization::FinishedFile,
    transfer_activity::Phase,
};
use anyhow::Context;
use reqwest::header;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipts {
    version: u32,
    binding: String,
    inputs: Vec<Option<String>>,
    outputs: Vec<Option<String>>,
}
fn part(root: &Path, index: usize, output: bool) -> PathBuf {
    root.join(format!(
        "provider-media-{index}.{}",
        if output { "out" } else { "in" }
    ))
}
fn save(root: &Path, receipts: &Receipts) -> Result<()> {
    let pending = root.join("provider-media.pending");
    let mut file = download_file::fresh_sync(&pending, false)?;
    file.write_all(&serde_json::to_vec(receipts)?)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(pending, root.join("provider-media.json"))?;
    Ok(())
}
fn hash(file: &mut File, size: u64, cancel: &CancellationToken) -> Result<String> {
    ensure!(
        file.metadata()?.len() == size,
        "Saved media segment size changed. Restart this download."
    );
    let mut digest = Sha256::new();
    let mut buffer = vec![0; CHUNK_BYTES];
    let mut read = 0;
    loop {
        ensure!(!cancel.is_cancelled(), "Media provider cancelled");
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        read += n as u64;
        ensure!(
            read <= size,
            "Saved media segment grew. Restart this download."
        );
        digest.update(&buffer[..n]);
    }
    ensure!(
        read == size,
        "Saved media segment shrank. Restart this download."
    );
    Ok(format!("{:x}", digest.finalize()))
}
fn expected(actual: &str, wanted: Option<&str>, label: &str) -> Result<()> {
    ensure!(
        wanted.is_none_or(|h| h == actual),
        "Media {label} checksum does not match. The file was not published."
    );
    Ok(())
}
fn load(root: &Path, job: &Job, content: &Plan, cancel: &CancellationToken) -> Result<Receipts> {
    let provider = job.provider.as_ref().context("Missing provider")?;
    let binding = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(
            API,
            &provider.script.body,
            provider.script.revision,
            content
        ))?)
    );
    let count = content.tracks.iter().map(|t| t.segments.len()).sum();
    let path = root.join("provider-media.json");
    let receipts = if path.try_exists()? {
        let mut bytes = Vec::new();
        download_file::read_part_sync(&path)?
            .take(MAX_PLAN_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= MAX_PLAN_BYTES,
            "Media checkpoint exceeds its size limit"
        );
        let saved: Receipts = serde_json::from_slice(&bytes)
            .context("Invalid media checkpoint. Restart this download.")?;
        ensure!(
            saved.version == API
                && saved.binding == binding
                && saved.inputs.len() == count
                && saved.outputs.len() == count,
            "Media checkpoint does not match this content. Restart this download."
        );
        saved
    } else {
        Receipts {
            version: API,
            binding,
            inputs: vec![None; count],
            outputs: vec![None; count],
        }
    };
    for (index, segment) in content.tracks.iter().flat_map(|t| &t.segments).enumerate() {
        ensure!(
            receipts.outputs[index].is_none() || receipts.inputs[index].is_some(),
            "Media output has no source receipt"
        );
        for (output, saved, wanted) in [
            (false, &receipts.inputs[index], &segment.input_sha256),
            (true, &receipts.outputs[index], &segment.output_sha256),
        ] {
            if let Some(saved) = saved {
                let mut file = download_file::read_part_sync(&part(root, index, output))
                    .context("Saved media segment is missing. Restart this download.")?;
                let actual = hash(&mut file, segment.size, cancel)?;
                ensure!(
                    &actual == saved,
                    "Saved media segment changed. Restart this download."
                );
                expected(
                    &actual,
                    wanted.as_deref(),
                    if output { "plaintext" } else { "source" },
                )?;
            }
        }
    }
    save(root, &receipts)?;
    Ok(receipts)
}

pub(crate) async fn download(
    store: &Store,
    job: &mut Job,
    root: &Path,
    plan: Plan,
    cancel: &CancellationToken,
    limiter: &Limiter,
) -> Result<Option<FinishedFile>> {
    plan.validate(true)?;
    let helper = crate::hls::merge::Helper::checked(cancel).await?;
    let content = plan.content();
    store.activity.set(&job.id, Phase::CheckingSaved);
    let (r, j, c, token) = (
        root.to_owned(),
        job.clone(),
        content.clone(),
        cancel.clone(),
    );
    let mut receipts = tokio::task::spawn_blocking(move || load(&r, &j, &c, &token)).await??;
    job.saved_path = None;
    job.total = Some(plan.size());
    job.downloaded = plan
        .tracks
        .iter()
        .flat_map(|t| &t.segments)
        .zip(&receipts.inputs)
        .filter(|(_, h)| h.is_some())
        .map(|(s, _)| s.size)
        .sum();
    store.checkpoint(job).await?;
    // One bounded streaming request at a time; no worker writes outlive pause.
    // Completed segments are reusable, while an interrupted segment is retried.
    for (index, segment) in plan.tracks.iter().flat_map(|t| &t.segments).enumerate() {
        if receipts.inputs[index].is_some() {
            continue;
        }
        store.activity.set(&job.id, Phase::Downloading);
        let value = receive(
            store,
            job,
            segment,
            &part(root, index, false),
            cancel,
            limiter,
        )
        .await?;
        let Some(value) = value else {
            return Ok(None);
        };
        receipts.inputs[index] = Some(value);
        let (r, saved) = (root.to_owned(), receipts.clone());
        tokio::task::spawn_blocking(move || save(&r, &saved)).await??;
        store.checkpoint(job).await?;
    }
    if cancel.is_cancelled() {
        return Ok(None);
    }
    store.activity.set(&job.id, Phase::Processing);
    let (s, j, r, c, token) = (
        store.clone(),
        job.clone(),
        root.to_owned(),
        content.clone(),
        cancel.clone(),
    );
    let tracks =
        tokio::task::spawn_blocking(move || process(&s, &j, &r, &c, &mut receipts, &token))
            .await??;
    mux::finish(store, job, root, &content, tracks, helper, cancel).await
}

async fn receive(
    store: &Store,
    job: &mut Job,
    segment: &Segment,
    path: &Path,
    cancel: &CancellationToken,
    limiter: &Limiter,
) -> Result<Option<String>> {
    limiter.check_quota().await?;
    let mut context = job.clone();
    context.public_only =
        job.public_only || crate::auth::origin(&job.url)? != crate::auth::origin(&segment.url)?;
    // Media providers receive no browser/saved server credentials implicitly.
    context.authentication = None;
    context.source_page = None;
    let client = crate::auth::DownloadClient::for_replacement(store, &context, &segment.url)?;
    let mut request = client
        .get(&segment.url)?
        .same_origin()
        .header(header::ACCEPT_ENCODING, "identity");
    if let Some(start) = segment.range_start {
        request = request.header(
            header::RANGE,
            format!("bytes={start}-{}", start + segment.size - 1),
        );
    }
    let mut response = tokio::select! { _ = cancel.cancelled() => return Ok(None), value = request.send() => value? };
    crate::auth::check_response(&response)?;
    crate::retry::response(&response)?;
    engine::identity(&response)?;
    if let Some(start) = segment.range_start {
        ensure!(
            response.status() == reqwest::StatusCode::PARTIAL_CONTENT,
            "Media server ignored the requested range"
        );
        let (a, b, _) = engine::range(&response)?;
        ensure!(
            a == start && b == start + segment.size - 1,
            "Media server returned the wrong range"
        );
    } else {
        ensure!(
            response.status() == reqwest::StatusCode::OK,
            "Unexpected media response status"
        );
    }
    ensure!(
        response
            .content_length()
            .is_none_or(|size| size == segment.size),
        "Media segment size changed"
    );
    let mut output = download_file::buffered(download_file::fresh(path).await?);
    let mut digest = Sha256::new();
    let mut received = 0;
    let mut updated = Instant::now();
    let outcome: Result<bool> = async {
        loop {
            if received != segment.size {
                limiter.check_quota().await?;
            }
            let chunk = tokio::select! {
                _ = cancel.cancelled() => return Ok(false),
                value = engine::network_wait(response.chunk()) => value?
            };
            let Some(chunk) = chunk else { break };
            limiter.record_payload(chunk.len()).await?;
            received += chunk.len() as u64;
            ensure!(
                received <= segment.size,
                "Media segment exceeds its declared size"
            );
            let mut remaining = chunk.as_ref();
            while !remaining.is_empty() {
                let Some(n) = limiter.acquire(&job.id, remaining.len(), cancel).await else {
                    return Ok(false);
                };
                output.write_all(&remaining[..n]).await?;
                digest.update(&remaining[..n]);
                job.downloaded += n as u64;
                remaining = &remaining[n..];
            }
            if updated.elapsed() >= Duration::from_millis(400) {
                store.checkpoint(job).await?;
                updated = Instant::now();
            }
        }
        if received != segment.size {
            return Err(crate::retry::incomplete(
                "Media segment ended before its declared size",
            ));
        }
        Ok(true)
    }
    .await;
    download_file::flush_and_sync(&mut output).await?;
    if !outcome? || cancel.is_cancelled() {
        return Ok(None);
    }
    let actual = format!("{:x}", digest.finalize());
    expected(&actual, segment.input_sha256.as_deref(), "source")?;
    Ok(Some(actual))
}

fn process(
    store: &Store,
    job: &Job,
    root: &Path,
    content: &Plan,
    receipts: &mut Receipts,
    cancel: &CancellationToken,
) -> Result<Vec<File>> {
    let provider = job.provider.as_ref().context("Missing provider")?;
    let extension =
        store.extension_context(&provider.script, crate::extensions::Reach::Processor)?;
    let mut worker = Worker::start(
        &store.paths,
        &provider.script,
        Role::Processor,
        extension.as_ref(),
        cancel,
    )?;
    let mut index = 0;
    let mut buffer = vec![0; CHUNK_BYTES];
    for track in &content.tracks {
        let mut track_offset = 0;
        for (segment_index, segment) in track.segments.iter().enumerate() {
            if receipts.outputs[index].is_none() {
                let mut input = download_file::read_part_sync(&part(root, index, false))?;
                // Recheck the guarded handle immediately before giving bytes to Lua.
                let actual = hash(&mut input, segment.size, cancel)?;
                ensure!(
                    Some(&actual) == receipts.inputs[index].as_ref(),
                    "Media source changed before processing"
                );
                use std::io::{Seek, SeekFrom};
                input.seek(SeekFrom::Start(0))?;
                let mut output = download_file::fresh_sync(&part(root, index, true), false)?;
                let context = Chunk {
                    content_id: content.content_id.clone(),
                    track_id: track.id.clone(),
                    kind: track.kind,
                    segment_id: segment.id.clone(),
                    segment_index,
                    track_offset,
                    size: segment.size,
                };
                let mut offset = 0;
                let mut digest = Sha256::new();
                while offset < segment.size {
                    ensure!(!cancel.is_cancelled(), "Media provider cancelled");
                    let n = (segment.size - offset).min(CHUNK_BYTES as u64) as usize;
                    input.read_exact(&mut buffer[..n])?;
                    let bytes = if segment.processing == Processing::Lua {
                        worker.transform_media(&context, &buffer[..n], offset, cancel)?
                    } else {
                        buffer[..n].to_vec()
                    };
                    ensure!(!cancel.is_cancelled(), "Media provider cancelled");
                    output.write_all(&bytes)?;
                    digest.update(&bytes);
                    offset += n as u64;
                }
                let actual = format!("{:x}", digest.finalize());
                expected(&actual, segment.output_sha256.as_deref(), "plaintext")?;
                output.sync_all()?;
                receipts.outputs[index] = Some(actual);
                save(root, receipts)?;
            }
            track_offset += segment.size;
            index += 1;
        }
    }
    worker.finish_media(content, cancel)?;
    drop(worker);
    store.activity.set(&job.id, Phase::Assembling);
    let mut tracks = Vec::new();
    index = 0;
    for (number, track) in content.tracks.iter().enumerate() {
        let mut output =
            download_file::fresh_sync(&root.join(format!("provider-track-{number}.part")), true)?;
        for segment in &track.segments {
            let mut input = download_file::read_part_sync(&part(root, index, true))?;
            let mut digest = Sha256::new();
            let mut count = 0;
            loop {
                ensure!(!cancel.is_cancelled(), "Media provider cancelled");
                let n = input.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                count += n as u64;
                ensure!(count <= segment.size, "Media plaintext grew before muxing");
                digest.update(&buffer[..n]);
                output.write_all(&buffer[..n])?;
            }
            ensure!(
                count == segment.size
                    && Some(format!("{:x}", digest.finalize())) == receipts.outputs[index],
                "Media plaintext changed before muxing"
            );
            index += 1;
        }
        output.sync_all()?;
        tracks.push(output);
    }
    Ok(tracks)
}
