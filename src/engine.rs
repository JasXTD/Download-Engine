mod finalization;
pub(crate) mod http;
pub(crate) mod segmented;

use crate::transfer_activity::Phase;
use crate::{
    auth::{self, DownloadClient},
    bandwidth::Limiter,
    download_file,
    model::{Job, Plan, Segment, Status},
    retry,
    store::Store,
    synchronization::{self, FinishedFile, RemoteFile},
};
use anyhow::{Context, Result, bail, ensure};
use reqwest::{Client, Response, header};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
#[cfg(test)]
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

const MIN_SEGMENT: u64 = 2 * 1024 * 1024;
pub fn client() -> Result<Client> {
    client_with_scope(None)
}
pub(crate) fn client_builder() -> reqwest::ClientBuilder {
    Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .retry(reqwest::retry::never())
        .user_agent(concat!("DownloadEngine/", env!("CARGO_PKG_VERSION")))
        .referer(false)
}
pub(crate) fn client_with_scope(scope: Option<String>) -> Result<Client> {
    client_from_builder(client_builder(), scope)
}
fn client_from_builder(builder: reqwest::ClientBuilder, scope: Option<String>) -> Result<Client> {
    Ok(builder
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            let url = attempt.url();
            if attempt.previous().len() >= 10 {
                return attempt.error("Too many redirects");
            }
            if !matches!(url.scheme(), "http" | "https")
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return attempt.error("Unsupported redirect URL");
            }
            if attempt
                .previous()
                .last()
                .is_some_and(|u| u.scheme() == "https")
                && url.scheme() == "http"
            {
                return attempt.error("Refusing an HTTPS to HTTP redirect");
            }
            if scope.as_ref().is_some_and(|origin| url.origin().ascii_serialization() != *origin) {
                return attempt.error("Authenticated downloads cannot redirect to another server. Use a direct link for the intended server.");
            }
            attempt.follow()
        }))
        .build()?)
}
// Bound only time spent awaiting the network. Reqwest's body read timeout also
// runs between polls, which would mistake a low speed cap for a stalled server.
pub(crate) async fn network_wait<T>(
    future: impl std::future::Future<Output = std::result::Result<T, reqwest::Error>>,
) -> Result<T> {
    tokio::time::timeout(Duration::from_secs(30), future)
        .await
        .map_err(|_| retry::incomplete("The server connection timed out."))?
        .map_err(retry::network)
}
pub fn parse_content_range(value: &str) -> Result<(u64, u64, u64)> {
    let text = value.strip_prefix("bytes ").context("Missing byte range")?;
    let (range, total) = text.split_once('/').context("Invalid content range")?;
    let (start, end) = range.split_once('-').context("Invalid content range")?;
    let (start, end, total): (u64, u64, u64) = (start.parse()?, end.parse()?, total.parse()?);
    ensure!(
        start <= end && end < total && total <= i64::MAX as u64,
        "Invalid content range bounds"
    );
    Ok((start, end, total))
}
pub(crate) fn range(response: &Response) -> Result<(u64, u64, u64)> {
    parse_content_range(
        response
            .headers()
            .get(header::CONTENT_RANGE)
            .context("Server omitted Content-Range")?
            .to_str()?,
    )
}

fn probe_size(response: &Response) -> Result<Option<u64>> {
    let value = response
        .headers()
        .get(header::CONTENT_RANGE)
        .context("Server omitted Content-Range")?
        .to_str()?;
    // HTTP permits an unknown complete length. It cannot define a resumable
    // plan, but the ordinary full-response path can still download it safely.
    if let Some(bounds) = value.strip_suffix("/*") {
        ensure!(
            bounds == "bytes 0-0",
            "Server returned the wrong probe range"
        );
        return Ok(None);
    }
    let (start, end, size) = parse_content_range(value)?;
    ensure!(
        start == 0 && end == 0,
        "Server returned the wrong probe range"
    );
    Ok(Some(size))
}
pub(crate) fn identity(response: &Response) -> Result<()> {
    if let Some(encoding) = response.headers().get(header::CONTENT_ENCODING) {
        ensure!(
            encoding.to_str()?.eq_ignore_ascii_case("identity"),
            "Server compressed a byte-range response; safe resume is unavailable"
        );
    }
    Ok(())
}
pub(crate) fn strong_etag(response: &Response) -> Option<String> {
    let value = response.headers().get(header::ETAG)?.to_str().ok()?;
    (value.starts_with('"') && value.ends_with('"') && value.len() >= 2).then(|| value.to_owned())
}

// Every file response uses the same checks, including a second GET after an
// unresumable probe and each range worker. Error MIME types describe the error
// body, so authentication and retry classification must take precedence.
fn check_file_response(response: &Response) -> Result<()> {
    auth::check_response(response)?;
    retry::response(response)?;
    if response.status().is_success() {
        ensure!(
            !response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| {
                    let mime = v.to_ascii_lowercase();
                    mime.contains("mpegurl") || mime.contains("dash+xml")
                }),
            "Use Preview video to choose a quality before adding this stream"
        );
    }
    Ok(())
}

pub(crate) fn part_path(root: &Path, index: usize) -> PathBuf {
    root.join(format!("segment-{index}.part"))
}
/// Only one scheduler owns a job. Cancellation is cooperative: in-flight file
/// writes finish before the paused state is committed or another worker starts.
pub async fn run(
    store: Store,
    mut job: Job,
    cancel: CancellationToken,
    limiter: Limiter,
) -> Result<()> {
    use crate::diagnostics::{self, Details, Event, Level};
    let _activity = store.activity.begin(&job.id);
    let diagnostic_job = uuid::Uuid::parse_str(&job.id).ok();
    diagnostics::record(
        Level::Info,
        Event::DownloadStarted,
        Details {
            job: diagnostic_job,
            attempt: Some(job.retry.attempts),
            ..Default::default()
        },
    );
    let limiter = limiter.with_store(&store)?;
    job.retry.next_at = None;
    job.quota_waiting = false;
    let outcome = transfer(&store, &mut job, &cancel, &limiter).await;
    let failure = outcome
        .as_ref()
        .err()
        .map(crate::diagnostics::Failure::error);
    let mut completed_guard = None;
    match outcome {
        Ok(Some(file)) => {
            job.status = Status::Complete;
            job.retry = retry::State::default();
            job.saved_path = Some(file.path);
            completed_guard = file.guard;
            job.error = None;
        }
        Ok(None) => {
            job.status = Status::Paused;
            job.error = None;
        }
        Err(error) => {
            if cancel.is_cancelled() {
                job.status = Status::Paused;
                job.error = None;
            } else if crate::quota::reached(&error) {
                job.quota_waiting = true;
                if job.plan.is_some()
                    || job.hls.is_some()
                    || job.ftp.is_some()
                    || job.downloaded == 0
                {
                    job.status = Status::Queued;
                    job.error = Some(error.to_string());
                } else {
                    job.status = Status::Paused;
                    job.error = Some("Data quota reached. This server cannot safely resume the file. Raise or disable the quota, then resume to restart this download.".into());
                }
            } else {
                retry::failed(&mut job, &error, chrono::Utc::now().timestamp_millis());
            }
        }
    }
    synchronization::finished(&mut job);
    let credentials_retired = job.status == Status::Complete && job.authentication.is_some();
    if let Err(error) = auth::save_finished(&store, &mut job) {
        diagnostics::record(
            Level::Error,
            Event::DownloadStateSaveFailed,
            Details {
                job: diagnostic_job,
                failure: Some(crate::diagnostics::Failure::error(&error)),
                ..Default::default()
            },
        );
        return Err(error);
    }
    let event = match job.status {
        Status::Complete => Event::DownloadCompleted,
        _ if job.quota_waiting => Event::DownloadQuotaHeld,
        Status::Paused => Event::DownloadPaused,
        _ if job.retry.next_at.is_some() => Event::DownloadRetryScheduled,
        _ => Event::DownloadFailed,
    };
    diagnostics::record(
        if event == Event::DownloadFailed {
            Level::Error
        } else if failure.is_some() {
            Level::Warn
        } else {
            Level::Info
        },
        event,
        Details {
            job: diagnostic_job,
            bytes: Some(job.downloaded),
            attempt: Some(job.retry.attempts),
            failure,
            ..Default::default()
        },
    );
    drop(completed_guard);
    if job.status == Status::Complete {
        // Completion is durable before partial files are removed.
        let paths = store.paths.clone();
        let _ =
            tokio::task::spawn_blocking(move || download_file::discard_partial_files(&paths, &job))
                .await;
    }
    if credentials_retired {
        auth::cleanup_pending(&store)
            .context("Download completed, but saved credential cleanup is pending")?;
    }
    Ok(())
}
async fn transfer(
    store: &Store,
    job: &mut Job,
    cancel: &CancellationToken,
    limiter: &Limiter,
) -> Result<Option<FinishedFile>> {
    limiter.check_quota().await?;
    if let Some(file) = synchronization::check(store, job, cancel).await? {
        return Ok(Some(file));
    }
    let root = store.paths.parts(&job.id)?;
    if let Some(file) = finalization::recover(store, job, &root, cancel).await? {
        return Ok(Some(file));
    }
    if crate::mega::is_url(&job.url) {
        // A previous publication intent may only have collided with an existing
        // name. With no matching recovery anchor, verify again before retrying.
        job.saved_path = None;
    }
    job.integrity.actual = None;
    let _cache_guards =
        download_file::prepare_parts(&store.paths.data.join("parts"), &root).await?;
    tokio::fs::create_dir_all(&job.directory)
        .await
        .context("Cannot create the download folder")?;
    if job.hls.is_some() {
        return crate::hls::transfer(store, job, &root, cancel, limiter).await;
    }
    if job.ftp.is_some() {
        return crate::ftp::transfer(store, job, &root, cancel, limiter).await;
    }
    if crate::mega::is_url(&job.url) {
        return crate::mega::transfer(store, job, &root, cancel, limiter).await;
    }
    if crate::wetransfer::is_url(&job.url) {
        return crate::wetransfer::transfer(store, job, &root, cancel, limiter).await;
    }
    ensure!(
        !crate::hls::is_playlist_url(&job.url) && !crate::hls::is_site_video(&job.url),
        "Use Preview video to choose a quality before adding this stream"
    );
    http::transfer(
        http::Source::Direct(DownloadClient::for_job(store, job)?),
        store,
        job,
        &root,
        cancel,
        limiter,
    )
    .await
}
async fn single(
    mut response: Response,
    store: &Store,
    job: &mut Job,
    root: &Path,
    cancel: &CancellationToken,
    limiter: &Limiter,
) -> Result<bool> {
    let mut file = download_file::buffered(download_file::fresh(&root.join("whole.part")).await?);
    store.activity.set(&job.id, Phase::Downloading);
    let mut checkpoint = std::time::Instant::now();
    let outcome = async {
        loop {
            if job.total != Some(job.downloaded) {
                limiter.check_quota().await?;
            }
            let chunk = tokio::select! {
                _=cancel.cancelled()=>return Ok(false),
                next=network_wait(response.chunk())=>next?
            };
            let Some(chunk) = chunk else {
                break;
            };
            limiter.record_payload(chunk.len()).await?;
            if let Some(size) = job.total {
                ensure!(
                    job.downloaded + chunk.len() as u64 <= size,
                    "Server sent more bytes than declared"
                );
            }
            let mut remaining = chunk.as_ref();
            while !remaining.is_empty() {
                let Some(n) = limiter.acquire(&job.id, remaining.len(), cancel).await else {
                    return Ok(false);
                };
                #[cfg(test)]
                storage_tests::before_write(&root.join("whole.part"), n)?;
                file.write_all(&remaining[..n]).await?;
                remaining = &remaining[n..];
                job.downloaded += n as u64;
                if checkpoint.elapsed() >= Duration::from_millis(500) {
                    store.checkpoint(job).await?;
                    checkpoint = std::time::Instant::now();
                }
            }
        }
        if let Some(size) = job.total
            && job.downloaded != size
        {
            return Err(retry::incomplete(
                "Download ended before all bytes arrived.",
            ));
        }
        job.total = Some(job.downloaded);
        Ok(true)
    }
    .await;
    // Quota holds, like manual pauses, must release ownership only after I/O.
    download_file::flush_and_sync(&mut file).await?;
    outcome
}
async fn finalize(
    store: &Store,
    job: &mut Job,
    root: &Path,
    cancel: &CancellationToken,
    segmented: bool,
) -> Result<Option<FinishedFile>> {
    let sources: Vec<PathBuf> = if segmented {
        job.plan
            .as_ref()
            .context("Missing segment plan")?
            .byte_order()?
            .into_iter()
            .map(|i| part_path(root, i))
            .collect()
    } else {
        vec![root.join("whole.part")]
    };
    finalize_sources(store, job, cancel, sources).await
}
pub(crate) async fn finalize_sources(
    store: &Store,
    job: &mut Job,
    cancel: &CancellationToken,
    sources: Vec<PathBuf>,
) -> Result<Option<FinishedFile>> {
    finalization::finish(store, job, cancel, sources, None).await
}
/// Bind publication to bytes already authenticated by a provider's file MAC.
pub(crate) async fn finalize_verified_source(
    store: &Store,
    job: &mut Job,
    cancel: &CancellationToken,
    source: PathBuf,
    sha256: &str,
) -> Result<Option<FinishedFile>> {
    finalization::finish(store, job, cancel, vec![source], Some(sha256)).await
}
pub(crate) async fn publish_without_overwrite(
    store: &Store,
    job: &mut Job,
    source: &Path,
) -> Result<PathBuf> {
    store.activity.set(&job.id, Phase::Saving);
    let name = job.name.clone();
    let directory = job.directory.clone();
    let original = Path::new(&name);
    let stem = original
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("download");
    let extension = original.extension().and_then(|s| s.to_str());
    for index in 0..10000 {
        let candidate = if index == 0 {
            name.to_string()
        } else {
            format!(
                "{stem} ({index}){}",
                extension.map(|e| format!(".{e}")).unwrap_or_default()
            )
        };
        let path = directory.join(candidate);
        // Write intent before publishing. If the process stops immediately
        // after hard_link, the staging and destination identities prove success.
        job.saved_path = Some(path.clone());
        store.save(job)?;
        match tokio::fs::hard_link(source,&path).await {
            Ok(())=>return Ok(path),
            Err(e) if e.kind()==std::io::ErrorKind::AlreadyExists=>continue,
            Err(e)=>return Err(e).context("Cannot publish the file safely. Choose a local folder on a filesystem that supports hard links (such as NTFS).")
        }
    }
    bail!("Too many files have this name")
}

#[cfg(test)]
mod storage_tests;

#[cfg(test)]
mod finalization_tests;

#[cfg(test)]
mod timeout_tests {
    use super::*;
    #[tokio::test(start_paused = true)]
    async fn network_timeout_excludes_deliberate_pacing_but_still_retries_stalls() {
        let limiter = Limiter::new(&crate::bandwidth::Limits {
            global: Some(1024),
            ..Default::default()
        })
        .unwrap();
        let cancel = CancellationToken::new();
        network_wait(async { Ok::<_, reqwest::Error>(()) })
            .await
            .unwrap();
        let mut bytes = 40 * 1024;
        while bytes > 0 {
            bytes -= limiter.acquire("a", bytes, &cancel).await.unwrap();
        }
        network_wait(async { Ok::<_, reqwest::Error>(()) })
            .await
            .unwrap();
        let error = network_wait(std::future::pending::<
            std::result::Result<(), reqwest::Error>,
        >())
        .await
        .unwrap_err();
        let mut job = Job::new(
            "https://example.com/a",
            None,
            std::env::temp_dir(),
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
        retry::failed(&mut job, &error, 0);
        assert_eq!(job.status, Status::Queued);
        assert_eq!(job.retry.attempts, 1);
    }
    #[tokio::test]
    async fn real_response_can_be_held_longer_than_network_timeout_between_chunks() {
        use tokio::{net::TcpListener, sync::oneshot};
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        let (release, wait) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut header = Vec::new();
            let mut byte = [0];
            while !header.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).await.unwrap();
                header.push(byte[0]);
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\na")
                .await
                .unwrap();
            wait.await.unwrap();
            stream.write_all(b"b").await.unwrap();
        });
        let mut response = network_wait(client().unwrap().get(url).send())
            .await
            .unwrap();
        assert_eq!(
            network_wait(response.chunk())
                .await
                .unwrap()
                .unwrap()
                .as_ref(),
            b"a"
        );
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(40)).await;
        tokio::time::resume();
        release.send(()).unwrap();
        assert_eq!(
            network_wait(response.chunk())
                .await
                .unwrap()
                .unwrap()
                .as_ref(),
            b"b"
        );
        assert!(network_wait(response.chunk()).await.unwrap().is_none());
        server.await.unwrap();
    }
}
