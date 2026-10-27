//! Adaptive byte-range workers. Workers always flush and release ownership
//! before their saved range can be split, resumed, or published.
use super::*;
use crate::model::MAX_SEGMENTS;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tokio::task::JoinSet;

async fn part_len(path: &Path) -> Result<u64> {
    match tokio::fs::metadata(path).await {
        Ok(meta) => {
            ensure!(meta.is_file(), "A partial download path is not a file");
            Ok(meta.len())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(e.into()),
    }
}
async fn bytes_on_disk(root: &Path, plan: &Plan) -> Result<u64> {
    plan.byte_order()?;
    let mut bytes = 0;
    for (index, segment) in plan.segments.iter().enumerate() {
        let len = part_len(&part_path(root, index)).await?;
        ensure!(
            len <= segment.end - segment.start + 1,
            "A partial file is larger than its expected range. Restart this download."
        );
        bytes += len;
    }
    Ok(bytes)
}

struct SegmentWorker {
    downloaded: Arc<AtomicU64>,
    cancel: Option<CancellationToken>,
}

/// The donor has stopped and flushed before this is called. Only its unwritten
/// tail changes ownership; existing files never move, shrink, or change index.
async fn split_tail(
    store: &Store,
    job: &mut Job,
    root: &Path,
    index: usize,
    workers: &mut Vec<SegmentWorker>,
) -> Result<()> {
    let mut updated = job.clone();
    let plan = updated.plan.as_mut().context("Missing range plan")?;
    let segment = &mut plan.segments[index];
    let saved = part_len(&part_path(root, index)).await?;
    ensure!(
        saved <= segment.end - segment.start + 1,
        "A partial file exceeds its range"
    );
    workers[index].downloaded.store(saved, Ordering::Relaxed);
    let remaining = segment.end - segment.start + 1 - saved;
    if remaining < 2 * MIN_SEGMENT {
        return Ok(());
    }
    let tail = Segment {
        start: segment.start + saved + remaining / 2,
        end: segment.end,
    };
    segment.end = tail.start - 1;
    match tokio::fs::symlink_metadata(part_path(root, plan.segments.len())).await {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
        Ok(_) => bail!("Unexpected partial file. Restart this download."),
    }
    plan.segments.push(tail);
    plan.byte_order()?;
    // Commit ownership before replacement workers write. A failed save leaves
    // the original in-memory layout and all existing partial files intact.
    store.save(&updated)?;
    job.plan = updated.plan;
    workers.push(SegmentWorker {
        downloaded: Arc::new(AtomicU64::new(0)),
        cancel: None,
    });
    Ok(())
}

use crate::transfer_activity::{Activity, SegmentSnapshot, SegmentState};

fn sync_activity_segments(store: &Store, job_id: &str, plan: &Plan, workers: &[SegmentWorker]) {
    let segments = plan
        .segments
        .iter()
        .enumerate()
        .map(|(index, segment)| {
            let downloaded = workers
                .get(index)
                .map_or(0, |w| w.downloaded.load(Ordering::Relaxed));
            let state = if downloaded > segment.end.saturating_sub(segment.start) {
                SegmentState::Completed
            } else if workers.get(index).is_some_and(|w| w.cancel.is_some()) {
                SegmentState::ReceivingData
            } else {
                SegmentState::Waiting
            };
            SegmentSnapshot {
                index,
                start: segment.start,
                end: segment.end,
                downloaded,
                state,
            }
        })
        .collect();
    store.activity.set_segments(job_id, segments);
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run(
    client: &DownloadClient,
    store: &Store,
    job: &mut Job,
    cancel: &CancellationToken,
    root: &Path,
    limiter: &Limiter,
    connection_limit: usize,
) -> Result<bool> {
    store.activity.set(&job.id, Phase::CheckingSaved);
    let plan = job.plan.as_ref().context("Missing range plan")?;
    job.downloaded = bytes_on_disk(root, plan).await?;
    let mut workers = Vec::with_capacity(plan.segments.len());
    for index in 0..plan.segments.len() {
        workers.push(SegmentWorker {
            downloaded: Arc::new(AtomicU64::new(part_len(&part_path(root, index)).await?)),
            cancel: None,
        });
    }
    sync_activity_segments(store, &job.id, plan, &workers);
    store.save(job)?;
    let progress = Arc::new(AtomicU64::new(job.downloaded));
    let workers_cancel = cancel.child_token();
    let mut tasks = JoinSet::new();
    let mut splitting = None;
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    let mut first_error = None;
    store.activity.set(&job.id, Phase::Downloading);
    loop {
        // Resume can leave many incomplete parts. Fill idle slots first and
        // never exceed the transfer's snapshotted connection policy.
        let plan = job.plan.as_ref().expect("Range plan retained");
        if !workers_cancel.is_cancelled() {
            for (index, worker) in workers.iter_mut().enumerate() {
                if tasks.len() == connection_limit {
                    break;
                }
                let segment = &plan.segments[index];
                let done = worker.downloaded.load(Ordering::Relaxed);
                if worker.cancel.is_some() || done == segment.end - segment.start + 1 {
                    continue;
                }
                let token = workers_cancel.child_token();
                worker.cancel = Some(token.clone());
                let future = download_segment(
                    client.clone(),
                    plan.clone(),
                    segment.clone(),
                    part_path(root, index),
                    done,
                    progress.clone(),
                    worker.downloaded.clone(),
                    token,
                    limiter.clone(),
                    job.id.clone(),
                    index,
                    store.activity.clone(),
                );
                tasks.spawn(async move { (index, future.await) });
            }
            if tasks.len() < connection_limit && splitting.is_none() && workers.len() < MAX_SEGMENTS
            {
                let donor = workers
                    .iter()
                    .enumerate()
                    .filter(|(_, worker)| worker.cancel.is_some())
                    .map(|(index, worker)| {
                        let segment = &plan.segments[index];
                        let left = (segment.end - segment.start + 1)
                            .saturating_sub(worker.downloaded.load(Ordering::Relaxed));
                        (index, left)
                    })
                    .filter(|(_, left)| *left >= 2 * MIN_SEGMENT)
                    .max_by_key(|(_, left)| *left);
                if let Some((index, _)) = donor {
                    splitting = Some(index);
                    workers[index]
                        .cancel
                        .as_ref()
                        .expect("Running donor")
                        .cancel();
                }
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
                    Some(Ok((index, result))) => {
                        workers[index].cancel = None;
                        if let Err(error) = result {
                            retry::merge_error(&mut first_error, error);
                            workers_cancel.cancel();
                        }
                        if splitting == Some(index) {
                            splitting = None;
                            if !workers_cancel.is_cancelled() {
                                match split_tail(store, job, root, index, &mut workers).await {
                                    Ok(()) => {
                                        if let Some(plan) = &job.plan {
                                            sync_activity_segments(store, &job.id, plan, &workers);
                                        }
                                    }
                                    Err(error) => {
                                        retry::merge_error(&mut first_error, error);
                                        workers_cancel.cancel();
                                    }
                                }
                            }
                        }
                    }
                    Some(Err(error)) => {
                        retry::merge_error(&mut first_error, error.into());
                        workers_cancel.cancel();
                    }
                    None => {}
                }
            }
        }
        // Drain cooperatively even on failure, so no writes outlive the paused
        // or retry state. Dropping the join set early would abort its workers.
    }
    let plan = job.plan.as_ref().expect("Range plan retained");
    job.downloaded = bytes_on_disk(root, plan).await?;
    if cancel.is_cancelled() {
        return Ok(false);
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    ensure!(
        job.downloaded == plan.size,
        "Some download segments are incomplete"
    );
    Ok(true)
}
#[allow(clippy::too_many_arguments)]
async fn download_segment(
    client: DownloadClient,
    plan: Plan,
    segment: Segment,
    path: PathBuf,
    done: u64,
    progress: Arc<AtomicU64>,
    segment_progress: Arc<AtomicU64>,
    cancel: CancellationToken,
    limiter: Limiter,
    job_id: String,
    index: usize,
    activity: Activity,
) -> Result<()> {
    let start = segment.start + done;
    limiter.check_quota().await?;
    activity.update_segment(&job_id, index, done, SegmentState::SendingGet);
    let mut response = tokio::select! {
        _=cancel.cancelled()=>return Ok(()),
        result=client.get(&plan.final_url)?.header(header::ACCEPT_ENCODING,"identity")
            .header(header::RANGE,format!("bytes={start}-{}",segment.end))
            .header(header::IF_RANGE,&plan.etag).send()=>result?
    };
    check_file_response(&response)?;
    ensure!(
        response.status() == reqwest::StatusCode::PARTIAL_CONTENT,
        "Server changed the file or ignored a range. Restart to download it safely."
    );
    identity(&response)?;
    ensure!(
        range(&response)? == (start, segment.end, plan.size),
        "Server returned an unexpected byte range"
    );
    ensure!(
        strong_etag(&response).as_deref() == Some(plan.etag.as_str()),
        "The remote file changed during download. Restart to get the new version."
    );
    ensure!(
        response.url().as_str() == plan.final_url,
        "The range request redirected to a different file"
    );
    let expected = segment.end - start + 1;
    if let Some(len) = response.content_length() {
        ensure!(
            len == expected,
            "Range length does not match Content-Length"
        );
    }
    let file = download_file::append_part(&path).await?;
    ensure!(
        file.metadata().await?.len() == done,
        "The partial file changed while the download was starting"
    );
    activity.update_segment(&job_id, index, done, SegmentState::ReceivingData);
    let mut file = download_file::buffered(file);
    let mut activity_update = std::time::Instant::now();
    let outcome = async {
        let mut received = 0;
        loop {
            if received != expected {
                limiter.check_quota().await?;
            }
            let next = tokio::select! {
                _=cancel.cancelled()=>return Ok(()),
                result=network_wait(response.chunk())=>result?
            };
            let Some(chunk) = next else {
                break;
            };
            limiter.record_payload(chunk.len()).await?;
            received += chunk.len() as u64;
            ensure!(
                received <= expected,
                "Server sent too many bytes for a segment"
            );
            let mut remaining = chunk.as_ref();
            while !remaining.is_empty() {
                let Some(n) = limiter.acquire(&job_id, remaining.len(), &cancel).await else {
                    return Ok(());
                };
                #[cfg(test)]
                super::storage_tests::before_write(&path, n)?;
                file.write_all(&remaining[..n]).await?;
                remaining = &remaining[n..];
                progress.fetch_add(n as u64, Ordering::Relaxed);
                let current = segment_progress.fetch_add(n as u64, Ordering::Relaxed) + n as u64;
                if activity_update.elapsed() >= Duration::from_millis(100) {
                    activity.update_segment(&job_id, index, current, SegmentState::ReceivingData);
                    activity_update = std::time::Instant::now();
                }
            }
        }
        if received != expected {
            return Err(retry::incomplete(
                "A segment ended before all bytes arrived.",
            ));
        }
        activity.update_segment(&job_id, index, done + received, SegmentState::Completed);
        Ok(())
    }
    .await;
    // Finish buffered I/O even on errors before releasing ownership of the
    // range; Tokio may still have a write pending after write_all returns.
    download_file::flush_and_sync(&mut file).await?;
    outcome
}
