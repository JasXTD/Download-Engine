//! Remote ZIP inventory only: bounded range reads, no extraction or execution.
mod directory;
use crate::{
    auth::{self, DownloadClient},
    engine,
    model::Job,
    store::Store,
};
use anyhow::{Context, Result, ensure};
use reqwest::{Response, StatusCode, header};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub const MAX_ENTRIES: usize = 10_000;
pub const MAX_DIRECTORY_BYTES: u64 = 4 * 1024 * 1024;
const TAIL_BYTES: u64 = 65_535 + 22 + 20 + 56;

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub size: u64,
    pub compressed_size: u64,
    pub directory: bool,
    pub encrypted: bool,
    pub unusual_path: bool,
}
#[derive(Clone, Debug)]
pub struct Preview {
    pub entries: Vec<Entry>,
    pub archive_size: u64,
    pub expanded_size: u64,
    pub metadata_bytes: u64,
}

/// The supplied job is a snapshot; it is never queued or modified. Existing
/// per-download authentication and the current proxy route are reused.
pub async fn inspect(store: &Store, job: &Job, cancel: &CancellationToken) -> Result<Preview> {
    let url = crate::model::validate_url(&job.url)?;
    ensure!(
        matches!(url.scheme(), "http" | "https") && job.hls.is_none(),
        "ZIP preview supports direct HTTP and HTTPS file links"
    );
    tokio::select! {
        biased;
        _ = cancel.cancelled() => anyhow::bail!("ZIP preview cancelled"),
        result = tokio::time::timeout(Duration::from_secs(60), inspect_inner(store, job)) =>
            result.map_err(|_| anyhow::anyhow!("ZIP preview timed out. Try again when the server responds."))?,
    }
}

struct Remote {
    client: DownloadClient,
    url: String,
    size: u64,
    validator: Option<(header::HeaderName, String)>,
    tail_start: u64,
    tail: Vec<u8>,
    metadata_bytes: u64,
}
impl Remote {
    async fn open(store: &Store, job: &Job) -> Result<Self> {
        let client = DownloadClient::for_job(store, job)?;
        let response = client
            .get(&job.url)?
            .header(header::ACCEPT_ENCODING, "identity")
            .header(header::RANGE, format!("bytes=-{TAIL_BYTES}"))
            .send()
            .await?;
        check_response(&response)?;
        let (start, end, size) = engine::range(&response)?;
        ensure!(
            start == size.saturating_sub(TAIL_BYTES) && end == size - 1 && size >= 22,
            "Server returned an incorrect ZIP tail range"
        );
        let url = response.url().to_string();
        let validator = engine::strong_etag(&response)
            .map(|v| (header::ETAG, v))
            .or_else(|| {
                response
                    .headers()
                    .get(header::LAST_MODIFIED)
                    .and_then(|v| v.to_str().ok())
                    .filter(|v| httpdate::parse_http_date(v).is_ok())
                    .map(|v| (header::LAST_MODIFIED, v.to_owned()))
            });
        let tail = body(response, end - start + 1).await?;
        Ok(Self {
            client,
            url,
            size,
            validator,
            tail_start: start,
            metadata_bytes: tail.len() as u64,
            tail,
        })
    }
    async fn bytes(&mut self, start: u64, length: u64) -> Result<Vec<u8>> {
        let end = start.checked_add(length).context("ZIP offset overflow")?;
        ensure!(
            length <= MAX_DIRECTORY_BYTES && end <= self.size,
            "ZIP metadata is outside the archive or exceeds the preview limit"
        );
        if length == 0 {
            return Ok(Vec::new());
        }
        if start >= self.tail_start {
            return Ok(self.tail
                [(start - self.tail_start) as usize..(end - self.tail_start) as usize]
                .to_vec());
        }
        let (kind, validator) = self.validator.as_ref().context("This server provides no file validator for a safe multi-request ZIP preview. Download the archive normally instead.")?;
        let response = self
            .client
            .get(&self.url)?
            .header(header::ACCEPT_ENCODING, "identity")
            .header(header::RANGE, format!("bytes={start}-{}", end - 1))
            .header(header::IF_RANGE, validator)
            .send()
            .await?;
        check_response(&response)?;
        ensure!(
            response.url().as_str() == self.url,
            "ZIP download location changed during preview"
        );
        ensure!(
            engine::range(&response)? == (start, end - 1, self.size),
            "ZIP size or requested range changed during preview"
        );
        ensure!(
            response.headers().get(kind).and_then(|v| v.to_str().ok()) == Some(validator.as_str()),
            "ZIP file changed during preview. Try again."
        );
        let bytes = body(response, length).await?;
        self.metadata_bytes += bytes.len() as u64;
        Ok(bytes)
    }
}
fn check_response(response: &Response) -> Result<()> {
    auth::check_response(response)?;
    ensure!(
        response.status() == StatusCode::PARTIAL_CONTENT,
        "ZIP preview requires HTTP byte-range support (server returned HTTP {}). The archive was not downloaded; use the normal download action instead.",
        response.status().as_u16()
    );
    engine::identity(response)
}
async fn body(mut response: Response, length: u64) -> Result<Vec<u8>> {
    ensure!(
        response.content_length().is_none_or(|n| n == length),
        "ZIP range has an incorrect Content-Length"
    );
    let mut result = Vec::with_capacity(length as usize);
    while let Some(chunk) = engine::network_wait(response.chunk()).await? {
        ensure!(
            chunk.len() <= length as usize - result.len(),
            "ZIP server exceeded the requested metadata range"
        );
        result.extend_from_slice(&chunk);
    }
    ensure!(
        result.len() as u64 == length,
        "ZIP metadata response ended early"
    );
    Ok(result)
}
async fn inspect_inner(store: &Store, job: &Job) -> Result<Preview> {
    let mut remote = Remote::open(store, job).await?;
    let end = directory::end(&remote.tail, remote.tail_start)?;
    let directory = if let Some(offset) = end.zip64_offset {
        let bytes = remote.bytes(offset, 56).await?;
        directory::zip64(&bytes, offset, &end)?
    } else {
        end.directory
    };
    directory.validate()?;
    let bytes = remote.bytes(directory.offset, directory.length).await?;
    let (entries, expanded_size) = directory::parse(&bytes, &directory)?;
    Ok(Preview {
        entries,
        archive_size: remote.size,
        expanded_size,
        metadata_bytes: remote.metadata_bytes,
    })
}
