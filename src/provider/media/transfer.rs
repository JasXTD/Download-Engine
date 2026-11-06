use super::*;
use crate::{
    bandwidth::Limiter,
    download_file, engine,
    model::Job,
    provider::{CHUNK_BYTES, process::Worker},
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    input_sizes: Vec<Option<u64>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    output_sizes: Vec<Option<u64>>,
}
impl Receipts {
    fn output_budget(&self, content: &Plan) -> Result<budget::OutputBudget> {
        let input = content
            .tracks
            .iter()
            .flat_map(|t| &t.segments)
            .enumerate()
            .try_fold(0_u64, |total, (index, segment)| {
                let size = if self.input_sizes.is_empty() {
                    segment.size
                } else {
                    self.input_sizes[index].unwrap_or(0)
                };
                total.checked_add(size).context("Media input size overflow")
            })?;
        let mut budget = budget::OutputBudget::new(input)?;
        for (index, segment) in content.tracks.iter().flat_map(|t| &t.segments).enumerate() {
            if self.outputs[index].is_some() {
                let size = if self.output_sizes.is_empty() {
                    segment.size
                } else {
                    self.output_sizes[index].context("Missing media output size")?
                };
                budget.include(size)?;
            }
        }
        Ok(budget)
    }
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
            input_sizes: if content.has_unknown_sizes() {
                vec![None; count]
            } else {
                Vec::new()
            },
            output_sizes: if content.needs_output_sizes() {
                vec![None; count]
            } else {
                Vec::new()
            },
        }
    };
    ensure!(
        if content.needs_output_sizes() {
            receipts.output_sizes.len() == count
        } else {
            receipts.output_sizes.is_empty()
        },
        "Media output-size receipts do not match the plan. Restart this download."
    );
    ensure!(
        if content.has_unknown_sizes() {
            receipts.input_sizes.len() == count
        } else {
            receipts.input_sizes.is_empty()
        },
        "Media input-size receipts do not match the plan. Restart this download."
    );
    for (index, segment) in content.tracks.iter().flat_map(|t| &t.segments).enumerate() {
        let input_size = if receipts.input_sizes.is_empty() {
            segment.size
        } else {
            ensure!(
                receipts.input_sizes[index].is_some() == receipts.inputs[index].is_some(),
                "Media input size has no matching receipt"
            );
            if let Some(size) = receipts.input_sizes[index] {
                ensure!(
                    size > 0
                        && size <= input_limit(segment)
                        && (segment.size == 0 || size == segment.size),
                    "Invalid saved media input size"
                );
            }
            receipts.input_sizes[index].unwrap_or(0)
        };
        if !receipts.output_sizes.is_empty() {
            ensure!(
                receipts.output_sizes[index].is_some() == receipts.outputs[index].is_some(),
                "Media output size has no matching receipt"
            );
            if let Some(size) = receipts.output_sizes[index] {
                ensure!(
                    size > 0
                        && if segment.processing == Processing::LuaSegment {
                            size <= MAX_SEGMENT_OUTPUT as u64
                        } else if segment.processing == Processing::LuaStream {
                            size <= MAX_STREAM_OUTPUT
                        } else {
                            size == input_size
                        },
                    "Invalid saved media output size"
                );
            }
        }
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
                let size = if output && !receipts.output_sizes.is_empty() {
                    receipts.output_sizes[index].context("Missing media output size")?
                } else {
                    input_size
                };
                let actual = hash(&mut file, size, cancel)?;
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
    // A checkpoint cannot reset the allowance or hide output on later tracks.
    receipts.output_budget(content)?;
    save(root, &receipts)?;
    Ok(receipts)
}

pub(crate) async fn download(
    store: &Store,
    job: &mut Job,
    root: &Path,
    plan: Plan,
    mut keys: crate::provider::handoff::Keys,
    cancel: &CancellationToken,
    limiter: &Limiter,
) -> Result<Option<FinishedFile>> {
    plan.validate(true)?;
    let sessions = keys.take_sessions()?;
    let helper = crate::hls::merge::Helper::checked(cancel).await?;
    let mut content = plan.content();
    store.activity.set(&job.id, Phase::CheckingSaved);
    let (r, j, c, token) = (
        root.to_owned(),
        job.clone(),
        content.clone(),
        cancel.clone(),
    );
    let mut receipts = tokio::task::spawn_blocking(move || load(&r, &j, &c, &token)).await??;
    job.saved_path = None;
    job.total = (!plan.has_unknown_sizes()).then(|| plan.size());
    job.downloaded = plan
        .tracks
        .iter()
        .flat_map(|t| &t.segments)
        .zip(&receipts.inputs)
        .enumerate()
        .filter(|(_, (_, h))| h.is_some())
        .map(|(index, (s, _))| {
            if receipts.input_sizes.is_empty() {
                s.size
            } else {
                receipts.input_sizes[index].unwrap_or(0)
            }
        })
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
            segment
                .session
                .as_ref()
                .map(|name| sessions.get(name).context("Missing media HTTP session"))
                .transpose()?
                .cloned(),
        )
        .await?;
        let Some(value) = value else {
            return Ok(None);
        };
        receipts.inputs[index] = Some(value.0);
        if !receipts.input_sizes.is_empty() {
            receipts.input_sizes[index] = Some(value.1);
        }
        let (r, saved) = (root.to_owned(), receipts.clone());
        tokio::task::spawn_blocking(move || save(&r, &saved)).await??;
        store.checkpoint(job).await?;
    }
    if cancel.is_cancelled() {
        return Ok(None);
    }
    // Persist the original plan, but give processors the actual verified byte sizes.
    if !receipts.input_sizes.is_empty() {
        for (segment, size) in content
            .tracks
            .iter_mut()
            .flat_map(|t| &mut t.segments)
            .zip(&receipts.input_sizes)
        {
            segment.size = size.context("Missing media input size")?;
        }
    }
    content.validate(false)?;
    job.total = Some(content.size());
    store.checkpoint(job).await?;
    store.activity.set(&job.id, Phase::Processing);
    let (s, mut j, r, c, token) = (
        store.clone(),
        job.clone(),
        root.to_owned(),
        content.clone(),
        cancel.clone(),
    );
    let (result, downloaded, total) = tokio::task::spawn_blocking(move || {
        let result = process(&s, &mut j, &r, &c, &mut receipts, keys, &token);
        (result, j.downloaded, j.total)
    })
    .await?;
    // Preserve processing progress even when cancellation or a budget check fails.
    job.downloaded = downloaded;
    job.total = total;
    let tracks = result?;
    mux::finish(store, job, root, &content, tracks, helper, cancel).await
}

fn input_limit(segment: &Segment) -> u64 {
    if segment.processing == Processing::LuaSegment {
        MAX_SEGMENT_INPUT as u64
    } else {
        crate::hls::MAX_PART_BYTES
    }
}

async fn receive(
    store: &Store,
    job: &mut Job,
    segment: &Segment,
    path: &Path,
    cancel: &CancellationToken,
    limiter: &Limiter,
    session: Option<crate::script_automation::HttpSession>,
) -> Result<Option<(String, u64)>> {
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
    if let Some(session) = session {
        request = request.http_session(session);
    }
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
    let expected_size = if segment.size > 0 {
        Some(segment.size)
    } else {
        response.content_length()
    };
    ensure!(
        expected_size.is_none_or(|size| size > 0 && size <= input_limit(segment)),
        "Media segment exceeds its input limit"
    );
    ensure!(
        response
            .content_length()
            .is_none_or(|size| segment.size == 0 || size == segment.size),
        "Media segment size changed"
    );
    let mut output = download_file::buffered(download_file::fresh(path).await?);
    let mut digest = Sha256::new();
    let mut received = 0;
    let mut updated = Instant::now();
    let outcome: Result<bool> = async {
        loop {
            if expected_size != Some(received) {
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
                received <= expected_size.unwrap_or_else(|| input_limit(segment)),
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
        if received == 0 || expected_size.is_some_and(|size| received != size) {
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
    Ok(Some((actual, received)))
}

fn process(
    store: &Store,
    job: &mut Job,
    root: &Path,
    content: &Plan,
    receipts: &mut Receipts,
    keys: crate::provider::handoff::Keys,
    cancel: &CancellationToken,
) -> Result<Vec<File>> {
    let mut budget = receipts.output_budget(content)?;
    job.downloaded = budget.used();
    job.total = None; // The output size is unknown until transforms finish.
    store.save(job)?;
    ensure!(!cancel.is_cancelled(), "Media provider cancelled");
    let provider = job.provider.as_ref().context("Missing provider")?;
    let extension =
        store.extension_context(&provider.script, crate::extensions::Reach::Processor)?;
    let mut worker = Worker::start_processor(
        &store.paths,
        &provider.script,
        extension.as_ref(),
        keys,
        cancel,
    )?;
    let mut index = 0;
    let mut buffer = vec![0; CHUNK_BYTES];
    for track in &content.tracks {
        let make_context = |segment: &Segment, segment_index, track_offset, output_offset| Chunk {
            content_id: content.content_id.clone(),
            track_id: track.id.clone(),
            kind: track.kind,
            segment_id: segment.id.clone(),
            segment_index,
            track_offset,
            output_offset,
            size: segment.size,
        };
        // Always replay the verified ORIGINAL initialization bytes, including when
        // their processed output was checkpointed by an earlier worker process.
        if track.init_segment.is_some() {
            let segment = &track.segments[0];
            let mut input = download_file::read_part_sync(&part(root, index, false))?;
            let actual = hash(&mut input, segment.size, cancel)?;
            ensure!(
                Some(&actual) == receipts.inputs[index].as_ref(),
                "Media initialization changed before processing"
            );
            use std::io::{Seek, SeekFrom};
            input.seek(SeekFrom::Start(0))?;
            worker.whole_segment(&make_context(segment, 0, 0, 0), &mut input, true, 0, cancel)?;
        }
        let mut track_offset = 0;
        let mut output_offset = 0;
        for (segment_index, segment) in track.segments.iter().enumerate() {
            if receipts.outputs[index].is_none() {
                ensure!(!cancel.is_cancelled(), "Media provider cancelled");
                let allowance = budget.allowance(segment.output_limit())?;
                if matches!(segment.processing, Processing::Copy | Processing::Lua) {
                    ensure!(
                        segment.size <= allowance,
                        "Media output exceeds its job-wide allowance"
                    );
                }
                budget::check_processing_space(
                    root,
                    &job.directory,
                    budget.used(),
                    allowance,
                    (segment.processing == Processing::LuaStream).then_some(segment.size),
                )?;
                let mut input = download_file::read_part_sync(&part(root, index, false))?;
                // Recheck the guarded handle immediately before giving bytes to Lua.
                let actual = hash(&mut input, segment.size, cancel)?;
                ensure!(
                    Some(&actual) == receipts.inputs[index].as_ref(),
                    "Media source changed before processing"
                );
                use std::io::{Seek, SeekFrom};
                input.seek(SeekFrom::Start(0))?;
                let mut output = download_file::fresh_sync(
                    &part(root, index, true),
                    segment.processing == Processing::LuaStream,
                )?;
                let context = make_context(segment, segment_index, track_offset, output_offset);
                let result = (|| -> Result<(String, u64)> {
                    let mut offset = 0;
                    let mut digest = Sha256::new();
                    let output_size = if segment.processing == Processing::LuaStream {
                        worker.stream_segment(
                            &context,
                            &mut input,
                            &mut budget::DiskWriter {
                                file: &mut output,
                                directory: root,
                            },
                            allowance,
                            cancel,
                        )?
                    } else if segment.processing == Processing::LuaSegment {
                        let bytes =
                            worker.whole_segment(&context, &mut input, false, allowance, cancel)?;
                        ensure!(!cancel.is_cancelled(), "Media provider cancelled");
                        budget::DiskWriter {
                            file: &mut output,
                            directory: root,
                        }
                        .write_all(&bytes)?;
                        digest.update(&bytes);
                        bytes.len() as u64
                    } else {
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
                            budget::DiskWriter {
                                file: &mut output,
                                directory: root,
                            }
                            .write_all(&bytes)?;
                            digest.update(&bytes);
                            offset += n as u64;
                        }
                        segment.size
                    };
                    let actual = if segment.processing == Processing::LuaStream {
                        // This branch writes directly to disk, so hash the guarded output.
                        output.sync_all()?;
                        output.seek(SeekFrom::Start(0))?;
                        hash(&mut output, output_size, cancel)?
                    } else {
                        format!("{:x}", digest.finalize())
                    };
                    expected(&actual, segment.output_sha256.as_deref(), "plaintext")?;
                    output.sync_all()?;
                    Ok((actual, output_size))
                })();
                let (actual, output_size) = match result {
                    Ok(value) => value,
                    Err(error) => {
                        // Truncate our held file, never a pathname that could be replaced.
                        output
                            .set_len(0)
                            .context("Cannot clear incomplete media output")?;
                        return Err(error);
                    }
                };
                budget.include(output_size)?;
                receipts.outputs[index] = Some(actual);
                if !receipts.output_sizes.is_empty() {
                    receipts.output_sizes[index] = Some(output_size);
                }
                save(root, receipts)?;
                job.downloaded = budget.used();
                store.save(job)?;
            }
            track_offset += segment.size;
            output_offset += if receipts.output_sizes.is_empty() {
                segment.size
            } else {
                receipts.output_sizes[index].context("Missing media output size")?
            };
            index += 1;
        }
    }
    let output_size = budget.used();
    job.downloaded = output_size;
    job.total = Some(output_size);
    store.save(job)?;
    worker.finish_media(content, output_size, cancel)?;
    drop(worker);
    store.activity.set(&job.id, Phase::Assembling);
    budget::check_processing_space(root, &job.directory, output_size, 0, None)?;
    let mut tracks = Vec::new();
    index = 0;
    for (number, track) in content.tracks.iter().enumerate() {
        let mut output =
            download_file::fresh_sync(&root.join(format!("provider-track-{number}.part")), true)?;
        for segment in &track.segments {
            let expected_size = if receipts.output_sizes.is_empty() {
                segment.size
            } else {
                receipts.output_sizes[index].context("Missing media output size")?
            };
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
                ensure!(count <= expected_size, "Media plaintext grew before muxing");
                digest.update(&buffer[..n]);
                budget::DiskWriter {
                    file: &mut output,
                    directory: root,
                }
                .write_all(&buffer[..n])?;
            }
            ensure!(
                count == expected_size
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

#[cfg(test)]
mod budget_tests {
    use super::*;
    #[test]
    fn resumed_unknown_size_receipts_count_saved_output_on_every_track() {
        let mut content = super::super::tests::plan().content();
        for segment in content.tracks.iter_mut().flat_map(|t| &mut t.segments) {
            segment.size = 0;
            segment.processing = Processing::LuaStream;
        }
        let saved = Receipts {
            version: API,
            binding: "fixture".into(),
            inputs: vec![Some("a".into()), Some("b".into())],
            outputs: vec![None, Some("saved-on-later-track".into())],
            input_sizes: vec![Some(1), Some(1)],
            output_sizes: vec![None, Some(16 * 1024 * 1024)],
        };
        let mut restored: Receipts =
            serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
        assert_eq!(
            restored
                .output_budget(&content)
                .unwrap()
                .allowance(MAX_STREAM_OUTPUT)
                .unwrap(),
            4
        );
        restored.outputs[0] = Some("new".into());
        restored.output_sizes[0] = Some(5);
        assert!(restored.output_budget(&content).is_err());
        restored.output_sizes[0] = Some(4);
        assert_eq!(
            restored.output_budget(&content).unwrap().used(),
            16 * 1024 * 1024 + 4
        );
    }
}
