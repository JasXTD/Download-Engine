//! Synchronization never replaces an existing file. A completed copy remains
//! the baseline until the usual verified, atomic download publication succeeds.
use crate::{
    auth::{self, DownloadClient},
    engine, integrity,
    model::{Job, Plan, Status},
    retry,
    store::Store,
};
use anyhow::{Context, Result, ensure};
use reqwest::{Response, StatusCode, header};
use serde::{Deserialize, Serialize};
use std::{fs::File, path::PathBuf};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteFile {
    pub final_url: String,
    pub etag: Option<String>,
    pub mime: Option<String>,
}
impl RemoteFile {
    pub(crate) fn from_response(response: &Response) -> Self {
        Self {
            final_url: response.url().to_string(),
            // Weak tags and modification dates do not establish byte identity.
            etag: engine::strong_etag(response),
            mime: mime(response),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Checking,
    Updating,
    UpToDate,
    Updated,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompletedCopy {
    pub path: PathBuf,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub plan: Option<Plan>,
    pub sha256: Option<String>,
    pub remote: Option<RemoteFile>,
    pub expected_mime: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct State {
    pub phase: Phase,
    pub previous: CompletedCopy,
    pub checked_at: Option<i64>,
}
impl State {
    pub(crate) fn begin(job: &mut Job) -> Result<()> {
        ensure!(
            job.ftp.is_none(),
            "FTP downloads do not support synchronization checks. Use Redownload instead."
        );
        ensure!(
            job.status == Status::Complete,
            "Finish the download before checking for updates"
        );
        let previous = CompletedCopy {
            path: job
                .saved_path
                .clone()
                .context("The saved file location is unavailable")?,
            downloaded: job.downloaded,
            total: job.total,
            plan: job.plan.clone(),
            sha256: job.integrity.actual.clone(),
            remote: job.remote.clone().or_else(|| {
                job.plan.as_ref().map(|plan| RemoteFile {
                    final_url: plan.final_url.clone(),
                    etag: Some(plan.etag.clone()),
                    mime: None,
                })
            }),
            expected_mime: job
                .expected_file
                .as_ref()
                .map(|expected| expected.mime.clone())
                .or_else(|| {
                    job.synchronization
                        .as_ref()
                        .and_then(|state| state.previous.expected_mime.clone())
                }),
        };
        job.synchronization = Some(Self {
            phase: Phase::Checking,
            previous,
            checked_at: None,
        });
        job.status = Status::Queued;
        job.error = None;
        job.retry = retry::State::default();
        Ok(())
    }

    pub fn label(&self, status: Status) -> &'static str {
        match status {
            Status::Complete if self.phase == Phase::UpToDate => "Up to date",
            Status::Complete => "Updated",
            Status::Failed => "Check failed",
            Status::Paused if self.phase == Phase::Checking => "Check paused",
            Status::Paused => "Update paused",
            Status::Queued => "Check queued",
            Status::Downloading if self.phase == Phase::Checking => "Checking",
            Status::Downloading => "Updating",
        }
    }
}

/// Keep the baseline read guard until the completion transaction, so a 304 or
/// matching hash cannot certify a file edited during the check (on Windows).
pub(crate) struct FinishedFile {
    pub path: PathBuf,
    pub guard: Option<File>,
}
async fn local_hash(
    copy: &CompletedCopy,
    cancel: &CancellationToken,
) -> Result<Option<(String, File)>> {
    let path = copy.path.clone();
    let token = cancel.clone();
    match tokio::task::spawn_blocking(move || integrity::hash_file(&path, &token)).await? {
        Ok(value) => Ok(Some(value)),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(None)
        }
        Err(error) => Err(error).context("Cannot verify the previous copy"),
    }
}

/// A conditional probe saves a body transfer only when both the saved bytes and
/// the final resource match. Without that proof, download and compare SHA-256.
pub(crate) async fn check(
    store: &Store,
    job: &mut Job,
    cancel: &CancellationToken,
) -> Result<Option<FinishedFile>> {
    let Some(state) = &job.synchronization else {
        return Ok(None);
    };
    if state.phase != Phase::Checking {
        return Ok(None);
    }
    let copy = state.previous.clone();
    if let Some((hash, guard)) = local_hash(&copy, cancel).await?
        && copy.sha256.as_ref() == Some(&hash)
        && let Some(remote) = &copy.remote
        && let Some(etag) = &remote.etag
    {
        let client = DownloadClient::for_job(store, job)?;
        let response = tokio::select! {
            _ = cancel.cancelled() => anyhow::bail!("Update check cancelled"),
            response = client.get(&job.url)?
                .header(header::ACCEPT_ENCODING, "identity")
                .header(header::CACHE_CONTROL, "no-cache")
                .header(header::RANGE, "bytes=0-0")
                .header(header::IF_NONE_MATCH, etag).send() => response?,
        };
        auth::check_response(&response)?;
        retry::response(&response)?;
        if response.status() == StatusCode::NOT_MODIFIED {
            ensure!(
                response.url().as_str() == remote.final_url,
                "The update check redirected to a different file; retry with the current download link"
            );
            ensure!(
                response
                    .headers()
                    .get(header::ETAG)
                    .is_none_or(|tag| tag == etag.as_str()),
                "The server returned conflicting file validators"
            );
            job.integrity.actual = Some(hash);
            job.integrity.check()?;
            mark_unchanged(job, None);
            return Ok(Some(FinishedFile {
                path: copy.path,
                guard: Some(guard),
            }));
        }
        ensure!(
            matches!(
                response.status(),
                StatusCode::OK | StatusCode::PARTIAL_CONTENT | StatusCode::RANGE_NOT_SATISFIABLE
            ),
            "Update check returned HTTP {}",
            response.status().as_u16()
        );
    }
    ensure!(!cancel.is_cancelled(), "Update check cancelled");
    // Clear only this job's owned staging data. Never touch the previous copy.
    let base = store.paths.data.join("parts");
    let root = store.paths.parts(&job.id)?;
    tokio::task::spawn_blocking(move || crate::download_file::reset_parts(&base, &root)).await??;
    let staging = job.directory.join(format!(".idm-ex-{}.assembling", job.id));
    match tokio::fs::remove_file(staging).await {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).context("Cannot reset the temporary download"),
    }
    job.plan = None;
    job.remote = None;
    job.saved_path = None;
    job.downloaded = 0;
    job.total = None;
    job.integrity.actual = None;
    // Browser size / ETag describe the old version. Preserve its MIME guard and
    // the user's expected SHA-256, but allow a new version to have a new size.
    job.expected_file = None;
    job.synchronization
        .as_mut()
        .expect("Synchronization check")
        .phase = Phase::Updating;
    store.save(job)?;
    Ok(None)
}

fn mime(response: &Response) -> Option<String> {
    response
        .headers()
        .get(header::CONTENT_TYPE)?
        .to_str()
        .ok()
        .map(|value| {
            value
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        })
}

pub(crate) fn check_response(job: &Job, response: &Response) -> Result<()> {
    if !matches!(
        response.status(),
        StatusCode::OK | StatusCode::PARTIAL_CONTENT
    ) {
        return Ok(());
    }
    if let Some(state) = &job.synchronization {
        let actual = mime(response).unwrap_or_else(|| "application/octet-stream".into());
        if let Some(expected) = &state.previous.expected_mime {
            ensure!(
                &actual == expected,
                "The update returned a different file type; check the download link or browser login"
            );
        }
        let previous = state
            .previous
            .remote
            .as_ref()
            .and_then(|remote| remote.mime.as_deref());
        if previous.is_some_and(|mime| !matches!(mime, "text/html" | "application/xhtml+xml")) {
            ensure!(
                !matches!(actual.as_str(), "text/html" | "application/xhtml+xml"),
                "The update returned a web page instead of the file; check the download link or browser login"
            );
        }
    }
    Ok(())
}

fn mark_unchanged(job: &mut Job, remote: Option<RemoteFile>) {
    let state = job.synchronization.as_mut().expect("Synchronization check");
    job.downloaded = state.previous.downloaded;
    job.total = state.previous.total;
    job.plan.clone_from(&state.previous.plan);
    job.remote = remote.or_else(|| state.previous.remote.clone());
    job.saved_path = Some(state.previous.path.clone());
    state.phase = Phase::UpToDate;
}

pub(crate) async fn reuse_unchanged(
    job: &mut Job,
    cancel: &CancellationToken,
) -> Result<Option<FinishedFile>> {
    let Some(state) = &job.synchronization else {
        return Ok(None);
    };
    let copy = state.previous.clone();
    if let Some((hash, guard)) = local_hash(&copy, cancel).await?
        && Some(&hash) == job.integrity.actual.as_ref()
    {
        let remote = job.remote.clone();
        mark_unchanged(job, remote);
        return Ok(Some(FinishedFile {
            path: copy.path,
            guard: Some(guard),
        }));
    }
    Ok(None)
}

/// Explicit restart, link replacement, or authentication changes must not reuse
/// a conditional check of the previous URL or a discarded download plan.
pub(crate) fn restarting(job: &mut Job) {
    job.remote = None;
    if let Some(state) = &mut job.synchronization {
        state.phase = Phase::Updating;
        job.saved_path = None;
    }
}

pub(crate) fn finished(job: &mut Job) {
    if let Some(state) = &mut job.synchronization {
        if job.status == Status::Complete && state.phase != Phase::UpToDate {
            state.phase = Phase::Updated;
        }
        if matches!(job.status, Status::Complete | Status::Failed) {
            state.checked_at = Some(chrono::Utc::now().timestamp());
        }
    }
}
