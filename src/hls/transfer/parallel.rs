//! Bounded video workers. Only flushed, validated parts enter durable recovery state.
use super::*;
use crate::{hls::Part, retry, transfer_activity::Activity};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tokio::io::AsyncWriteExt;
use tokio::task::JoinSet;

const MAX_WORKERS: usize = 4;

#[allow(clippy::too_many_arguments)]
pub(super) async fn run(
    network: Network,
    store: &Store,
    job: &mut Job,
    root: &Path,
    cancel: &CancellationToken,
    limiter: &Limiter,
    parts: Vec<(Part, Format)>,
) -> Result<bool> {
    let connections = store.connection_settings()?;
    let playlist = &job.hls.as_ref().context("Missing video download")?.playlist;
    let mut limit = MAX_WORKERS;
    for (part, _) in &parts {
        limit = limit.min(connections.limit(&playlist.source, &part.url)?);
    }
    let video_count = playlist.parts.len();
    let dash = Arc::new(playlist.dash.clone());
    let parts = Arc::new(parts);
    let mut assigned = vec![false; parts.len()];
    let progress = Arc::new(AtomicU64::new(job.downloaded));
    let workers_cancel = cancel.child_token();
    let mut tasks = JoinSet::new();
    let mut first_error = None;
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    store
        .activity
        .set(&job.id, crate::transfer_activity::Phase::Downloading);
    loop {
        if !workers_cancel.is_cancelled() {
            let saved = job.hls.as_ref().unwrap();
            // Bound out-of-order receipts and disk usage behind a slow part.
            let end = (saved.completed.len() + 2 * limit).min(parts.len());
            for index in saved.completed.len()..end {
                if tasks.len() >= limit {
                    break;
                }
                if assigned[index] || saved.saved_part(index).is_some() {
                    continue;
                }
                let first = if index < video_count { 0 } else { video_count };
                // Verify a track's first part/init before requesting its media.
                // This retains the encryption/container gate on parallel paths.
                if index != first && saved.saved_part(first).is_none() {
                    continue;
                }
                assigned[index] = true;
                let (part, format) = parts[index].clone();
                let (network, limiter, token, activity, progress, dash, id, path) = (
                    network.clone(),
                    limiter.clone(),
                    workers_cancel.clone(),
                    store.activity.clone(),
                    progress.clone(),
                    dash.clone(),
                    job.id.clone(),
                    part_path(root, index),
                );
                tasks.spawn(async move {
                    (
                        index,
                        receive(
                            network, &id, index, part, format, path, token, limiter, activity,
                            progress, dash,
                        )
                        .await,
                    )
                });
            }
        }
        if tasks.is_empty() {
            break;
        }
        tokio::select! {
            _ = tick.tick() => {
                job.downloaded = progress.load(Ordering::Relaxed);
                if let Err(error) = store.checkpoint(job).await {
                    retry::merge_error(&mut first_error, error);
                    workers_cancel.cancel();
                }
            }
            result = tasks.join_next() => {
                match result {
                    Some(Ok((index, Ok(Some(part))))) => {
                        job.downloaded = progress.load(Ordering::Relaxed);
                        let saved = job.hls.as_mut().unwrap().remember_part(index, part);
                        let saved = match saved { Ok(()) => store.checkpoint(job).await, Err(error) => Err(error) };
                        if let Err(error) = saved {
                            retry::merge_error(&mut first_error, error);
                            workers_cancel.cancel();
                        }
                        store.activity.remove_media(&job.id, index);
                    }
                    Some(Ok((_, Ok(None)))) => { workers_cancel.cancel(); }
                    Some(Ok((_, Err(error)))) => {
                        retry::merge_error(&mut first_error, error);
                        workers_cancel.cancel();
                    }
                    Some(Err(error)) => {
                        retry::merge_error(&mut first_error, error.into());
                        workers_cancel.cancel();
                    }
                    None => {}
                }
            }
        }
    }
    // Join every worker before returning: no writes may outlive pause/retry.
    job.downloaded = progress.load(Ordering::Relaxed);
    if let Some(error) = first_error {
        return Err(error);
    }
    if workers_cancel.is_cancelled() {
        return Ok(false);
    }
    ensure!(
        job.hls.as_ref().unwrap().completed.len() == parts.len(),
        "Some video parts are incomplete"
    );
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
async fn receive(
    network: Network,
    id: &str,
    index: usize,
    part: Part,
    format: Format,
    path: PathBuf,
    cancel: CancellationToken,
    limiter: Limiter,
    activity: Activity,
    progress: Arc<AtomicU64>,
    dash: Arc<Option<crate::dash::Selection>>,
) -> Result<Option<CompletedPart>> {
    limiter.check_quota().await?;
    let mut live = MediaSnapshot {
        index,
        downloaded: 0,
        total: part.range.as_ref().map(|range| range.length),
        state: SegmentState::SendingGet,
    };
    activity.set_media(id, live);
    let response = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Ok(None),
        result = network.get(&part.url, part.range.as_ref(), &cancel) => result,
    };
    if cancel.is_cancelled() {
        return Ok(None);
    }
    let mut response = response?;
    if let Some(dash) = dash.as_ref() {
        for saved in dash.video_index.iter().chain(&dash.audio_index) {
            saved.check_response(&part, &response)?;
        }
    }
    let expected = part
        .range
        .as_ref()
        .map(|range| range.length)
        .or(response.content_length());
    ensure!(
        expected.is_none_or(|size| size > 0 && size <= MAX_PART_BYTES),
        "HLS segment exceeds 256 MiB or is empty"
    );
    live.total = expected;
    live.state = SegmentState::ReceivingData;
    activity.set_media(id, live);
    let mut file = download_file::buffered(download_file::fresh(&path).await?);
    let mut received = 0u64;
    let mut hash = Sha256::new();
    let mut updated = Instant::now();
    let result: Result<bool> = async {
        loop {
            if expected != Some(received) {
                limiter.check_quota().await?;
            }
            let chunk = tokio::select! {
                _ = cancel.cancelled() => return Ok(false),
                result = engine::network_wait(response.chunk()) => result?,
            };
            let Some(chunk) = chunk else {
                break;
            };
            limiter.record_payload(chunk.len()).await?;
            received += chunk.len() as u64;
            ensure!(
                received <= expected.unwrap_or(MAX_PART_BYTES),
                "HLS segment is larger than declared or exceeds 256 MiB"
            );
            let mut remaining = chunk.as_ref();
            while !remaining.is_empty() {
                let Some(n) = limiter.acquire(id, remaining.len(), &cancel).await else {
                    return Ok(false);
                };
                file.write_all(&remaining[..n]).await?;
                hash.update(&remaining[..n]);
                progress.fetch_add(n as u64, Ordering::Relaxed);
                live.downloaded += n as u64;
                remaining = &remaining[n..];
                if updated.elapsed() >= Duration::from_millis(100) {
                    activity.set_media(id, live);
                    updated = Instant::now();
                }
            }
        }
        if expected.is_some_and(|size| size != received) {
            return Err(retry::incomplete(
                "HLS segment ended before all bytes arrived.",
            ));
        }
        ensure!(received > 0, "HLS segment is empty");
        Ok(true)
    }
    .await;
    download_file::flush_and_sync(&mut file).await?;
    drop(file);
    if !result? || cancel.is_cancelled() {
        return Ok(None);
    }
    live.state = SegmentState::Verifying;
    activity.set_media(id, live);
    let validation = validate_media(&path, format, part.initialization, index == 0, &cancel).await;
    if cancel.is_cancelled() {
        return Ok(None);
    }
    validation?;
    live.state = SegmentState::Completed;
    activity.set_media(id, live);
    Ok(Some(CompletedPart {
        size: received,
        sha256: format!("{:x}", hash.finalize()),
    }))
}
