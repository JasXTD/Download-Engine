//! Replacement links are checked read-only. ETags identify a representation of
//! one resource, not identity across URLs (RFC 9110 section 8.8.1), so every saved
//! byte is compared before adopting a new validator. Credentials are reused only
//! for the bound origin and retired when accepting a different server.
use crate::{
    auth::{self, DownloadClient},
    bandwidth::Limiter,
    download_file, engine,
    model::{Job, Plan, Status, validate_url},
    store::Store,
};
use anyhow::{Context, Result, bail, ensure};
use reqwest::{Response, StatusCode, header};
use std::{
    fs::File,
    sync::mpsc,
    time::{Duration, Instant},
};
use tokio::{fs::File as AsyncFile, io::AsyncReadExt, task::JoinSet};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Debug)]
pub enum Command {
    CheckBrowser {
        request: Uuid,
        recovery: Uuid,
    },
    Check {
        request: Uuid,
        job: String,
        url: String,
    },
    Apply {
        request: Uuid,
        restart: bool,
    },
    Cancel(Uuid),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Resume { saved: u64, total: u64 },
    Fresh,
    RestartRequired(String),
}
#[derive(Debug)]
pub enum Event {
    Progress {
        request: Uuid,
        checked: u64,
        total: u64,
    },
    Checked {
        request: Uuid,
        outcome: Outcome,
    },
    Applied {
        request: Uuid,
        warning: Option<String>,
    },
    Failed {
        request: Uuid,
        message: String,
    },
}

struct Part {
    start: u64,
    len: u64,
    file: AsyncFile,
}
pub(crate) struct Prepared {
    recovery: Option<Uuid>,
    original: Job,
    url: String,
    plan: Option<Plan>,
    outcome: Outcome,
    // Retain read-only, no-write/no-delete handles through confirmation.
    parts: Vec<Part>,
    guards: Vec<File>,
}

async fn optional_part(path: &std::path::Path, start: u64) -> Result<Option<Part>> {
    match download_file::read_part(path).await {
        Ok(file) => Ok(Some(Part {
            start,
            len: file.metadata().await?.len(),
            file,
        })),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

async fn check(
    store: &Store,
    job: Job,
    url: String,
    cancel: &CancellationToken,
    limiter: &Limiter,
    progress: impl Fn(u64, u64),
) -> Result<Prepared> {
    ensure!(
        job.ftp.is_none(),
        "Add a new FTP download to use a different file link."
    );
    ensure!(
        job.hls.is_none(),
        "Preview the HLS video again to replace an expired playlist"
    );
    let limiter = limiter.with_store(store)?;
    limiter.check_quota().await?;
    // Cancel drops the HTTP response immediately. File operations are read-only;
    // Tokio may finish an outstanding read before releasing its guarded handle.
    tokio::select! {
        biased;
        _ = cancel.cancelled() => bail!("Link check cancelled"),
        result = check_inner(store, job, url, cancel, &limiter, progress) => result,
    }
}

async fn check_inner(
    store: &Store,
    job: Job,
    url: String,
    cancel: &CancellationToken,
    limiter: &Limiter,
    progress: impl Fn(u64, u64),
) -> Result<Prepared> {
    let root = store.paths.parts(&job.id)?;
    let guards = download_file::prepare_parts(&store.paths.data.join("parts"), &root).await?;
    let mut parts = Vec::new();
    if let Some(plan) = &job.plan {
        for index in plan.byte_order()? {
            let segment = &plan.segments[index];
            if let Some(part) =
                optional_part(&engine::part_path(&root, index), segment.start).await?
            {
                ensure!(
                    part.len <= segment.end - segment.start + 1,
                    "Saved data exceeds its range. Restart this download."
                );
                parts.push(part);
            }
        }
    } else if let Some(part) = optional_part(&root.join("whole.part"), 0).await? {
        parts.push(part);
    }
    let saved: u64 = parts.iter().map(|p| p.len).sum();
    progress(0, saved);
    let client = DownloadClient::for_replacement(store, &job, &url)?;
    let mut probe = client
        .get(&url)?
        .header(header::ACCEPT_ENCODING, "identity")
        .header(header::RANGE, "bytes=0-0")
        .send()
        .await?;
    if probe.status() == StatusCode::RANGE_NOT_SATISFIABLE {
        // Empty files cannot satisfy bytes=0-0. Check a normal GET instead.
        probe = client
            .get(&url)?
            .header(header::ACCEPT_ENCODING, "identity")
            .send()
            .await?;
    }
    auth::check_response(&probe)?;
    ensure!(
        matches!(probe.status(), StatusCode::OK | StatusCode::PARTIAL_CONTENT),
        "The replacement link returned HTTP {}",
        probe.status().as_u16()
    );
    engine::identity(&probe)?;
    let mime = probe
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    ensure!(
        !mime.eq_ignore_ascii_case("text/html")
            && !mime.eq_ignore_ascii_case("application/xhtml+xml"),
        "This link returns a web page. Paste a direct file link instead."
    );
    let ranged = probe.status() == StatusCode::PARTIAL_CONTENT;
    let size = if ranged {
        let (start, end, size) = engine::range(&probe)?;
        ensure!(
            start == 0 && end == 0,
            "The replacement returned an invalid probe range"
        );
        Some(size)
    } else {
        probe.content_length()
    };
    ensure!(
        size.is_none_or(|s| s <= i64::MAX as u64),
        "Replacement file is too large"
    );
    let etag = engine::strong_etag(&probe);
    let final_url = probe.url().to_string();
    if ranged {
        // A malformed/truncated probe is a failed check, never permission to restart.
        let mut received = 0;
        while let Some(chunk) = engine::network_wait(probe.chunk()).await? {
            received += chunk.len();
            ensure!(received <= 1, "The replacement returned an oversized probe");
        }
        ensure!(
            received == 1,
            "The replacement returned an incomplete probe"
        );
    }
    drop(probe);
    let mut prepared = Prepared {
        recovery: None,
        original: job,
        url,
        plan: None,
        outcome: Outcome::Fresh,
        parts,
        guards,
    };
    if saved == 0 {
        return Ok(prepared);
    }
    let reason = if prepared.original.plan.is_none() {
        Some("The saved download has no safe resume information.")
    } else if size != prepared.original.plan.as_ref().map(|p| p.size) {
        Some("The replacement file has a different or unknown size.")
    } else if !ranged || etag.is_none() {
        Some("This server cannot safely resume the saved download.")
    } else {
        None
    };
    if let Some(reason) = reason {
        prepared.outcome = Outcome::RestartRequired(reason.into());
        return Ok(prepared);
    }
    let mut plan = prepared
        .original
        .plan
        .clone()
        .context("Missing resume plan")?;
    plan.etag = etag.context("Missing file validator")?;
    plan.final_url = final_url;
    let mut checked = 0;
    let mut last_progress = Instant::now();
    let mut buffer = vec![0; 256 * 1024];
    for part in &mut prepared.parts {
        if part.len == 0 {
            continue;
        }
        let end = part.start + part.len - 1;
        limiter.check_quota().await?;
        let mut response = client
            .get(&plan.final_url)?
            .header(header::ACCEPT_ENCODING, "identity")
            .header(header::RANGE, format!("bytes={}-{}", part.start, end))
            .header(header::IF_RANGE, &plan.etag)
            .send()
            .await?;
        verify_range(&response, part.start, end, &plan)?;
        let mut received = 0;
        loop {
            if received != part.len {
                limiter.check_quota().await?;
            }
            let Some(chunk) = engine::network_wait(response.chunk()).await? else {
                break;
            };
            limiter.record_payload(chunk.len()).await?;
            ensure!(
                chunk.len() as u64 <= part.len - received,
                "The replacement sent too much data during verification"
            );
            let mut remaining = chunk.as_ref();
            while !remaining.is_empty() {
                let n = limiter
                    .acquire(
                        &prepared.original.id,
                        remaining.len().min(buffer.len()),
                        cancel,
                    )
                    .await
                    .context("Link check cancelled")?;
                let bytes = &remaining[..n];
                remaining = &remaining[n..];
                part.file
                    .read_exact(&mut buffer[..bytes.len()])
                    .await
                    .context("Cannot read the saved download")?;
                if buffer[..bytes.len()] != *bytes {
                    prepared.outcome = Outcome::RestartRequired(
                        "The replacement does not match the saved data.".into(),
                    );
                    return Ok(prepared);
                }
                received += bytes.len() as u64;
                checked += bytes.len() as u64;
                if last_progress.elapsed() >= Duration::from_millis(250) {
                    progress(checked, saved);
                    last_progress = Instant::now();
                }
            }
        }
        ensure!(
            received == part.len,
            "The replacement ended before verification finished"
        );
    }
    progress(checked, saved);
    prepared.outcome = Outcome::Resume {
        saved,
        total: plan.size,
    };
    prepared.plan = Some(plan);
    Ok(prepared)
}

fn verify_range(response: &Response, start: u64, end: u64, plan: &Plan) -> Result<()> {
    auth::check_response(response)?;
    ensure!(
        response.status() == StatusCode::PARTIAL_CONTENT,
        "The replacement stopped supporting verified ranges (HTTP {})",
        response.status().as_u16()
    );
    engine::identity(response)?;
    ensure!(
        engine::range(response)? == (start, end, plan.size),
        "The replacement returned an incorrect file range"
    );
    ensure!(
        engine::strong_etag(response).as_deref() == Some(&plan.etag)
            && response.url().as_str() == plan.final_url,
        "The replacement changed during verification. Check the link again."
    );
    ensure!(
        response
            .content_length()
            .is_none_or(|n| n == end - start + 1),
        "The replacement returned an incorrect range length"
    );
    Ok(())
}

impl Prepared {
    fn apply(self, store: &Store, restart: bool) -> Result<Option<String>> {
        let must_restart = matches!(self.outcome, Outcome::RestartRequired(_));
        ensure!(
            restart == must_restart,
            "Confirm the checked action before updating the link"
        );
        let mut conn = store.connect()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let data: String = tx
            .query_row(
                "SELECT data FROM jobs WHERE id=?1",
                [&self.original.id],
                |r| r.get(0),
            )
            .context("Download no longer exists")?;
        ensure!(
            serde_json::from_str::<serde_json::Value>(&data)?
                == serde_json::to_value(&self.original)?,
            "The download changed while checking the link. Check it again."
        );
        let mut job = self.original;
        if let Some(id) = self.recovery {
            job.source_page = Some(crate::browser_recovery::accept(
                &tx, id, &job.id, &self.url,
            )?);
        }
        // The scheduler owns this inactive job. Guards remain live through the
        // stale-state check; release them before an explicitly requested reset.
        drop(self.parts);
        drop(self.guards);
        if self.plan.is_none() {
            crate::download_file::discard_partial_files(&store.paths, &job)?;
        }
        if job
            .authentication
            .as_ref()
            .is_some_and(|b| auth::origin(&self.url).ok().as_deref() != Some(&b.origin))
            && let Some(binding) = job.authentication.take()
        {
            auth::retire(&tx, &binding)?;
        }
        job.retry = crate::retry::State::default();
        job.url = self.url;
        crate::synchronization::restarting(&mut job);
        // The replacement dialog promises to retain the selected filename.
        job.server_name_pending = false;
        job.plan = self.plan;
        job.downloaded = match self.outcome {
            Outcome::Resume { saved, .. } => saved,
            _ => 0,
        };
        job.total = job.plan.as_ref().map(|p| p.size);
        // Old browser validators describe the expired URL. The verified plan
        // now pins the replacement, or an explicit restart fetches it afresh.
        job.expected_file = None;
        job.integrity.actual = None;
        job.error = None;
        job.status = Status::Queued;
        tx.execute(
            "UPDATE jobs SET data=?1 WHERE id=?2",
            rusqlite::params![serde_json::to_string(&job)?, job.id],
        )?;
        tx.execute(
            "UPDATE job_queues SET paused_by_queue=0 WHERE job_id=?1",
            [&job.id],
        )?;
        crate::queue_start::wake_idle_main(&tx, &job.id)?;
        tx.commit()?;
        Ok(auth::cleanup_pending(store).err().map(|_| {
            "Link updated. Windows credential cleanup will retry when the app starts.".into()
        }))
    }
}

struct Session {
    request: Uuid,
    job: String,
    cancel: CancellationToken,
    prepared: Option<Prepared>,
}

/// One dialog/check at a time. The scheduler keeps this job inactive until all
/// reads finish, even when the user cancels or the app shuts down.
pub(crate) struct Controller {
    session: Option<Session>,
    tasks: JoinSet<Result<Prepared>>,
    events: mpsc::Sender<Event>,
    limiter: Limiter,
}
impl Controller {
    pub(crate) fn new(events: mpsc::Sender<Event>, limiter: Limiter) -> Self {
        Self {
            session: None,
            tasks: JoinSet::new(),
            events,
            limiter,
        }
    }
    pub(crate) fn busy(&self, job: &str) -> bool {
        self.session.as_ref().is_some_and(|s| s.job == job)
    }
    pub(crate) fn running(&self) -> bool {
        !self.tasks.is_empty()
    }
    pub(crate) async fn next(
        &mut self,
    ) -> Option<Result<Result<Prepared>, tokio::task::JoinError>> {
        self.tasks.join_next().await
    }
    pub(crate) fn complete(
        &mut self,
        result: Option<Result<Result<Prepared>, tokio::task::JoinError>>,
    ) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        if session.cancel.is_cancelled() {
            self.session = None;
            return;
        }
        let result = match result {
            Some(Ok(result)) => result,
            _ => Err(anyhow::anyhow!(
                "Link check stopped unexpectedly. Try again."
            )),
        };
        match result {
            Ok(prepared) => {
                let _ = self.events.send(Event::Checked {
                    request: session.request,
                    outcome: prepared.outcome.clone(),
                });
                session.prepared = Some(prepared);
            }
            Err(error) => {
                let _ = self.events.send(Event::Failed {
                    request: session.request,
                    message: format!("{error:#}"),
                });
                self.session = None;
            }
        }
    }
    pub(crate) fn handle(
        &mut self,
        store: &Store,
        command: Command,
        active: impl Fn(&str) -> bool,
    ) {
        let request = match &command {
            Command::Check { request, .. }
            | Command::CheckBrowser { request, .. }
            | Command::Apply { request, .. }
            | Command::Cancel(request) => *request,
        };
        if let Err(error) = self.apply_command(store, command, active) {
            // Failed is terminal for this request. Do not retain an invisible
            // reservation if an invalid early confirmation races with checking.
            if let Some(session) = &self.session
                && session.request == request
            {
                session.cancel.cancel();
                if self.tasks.is_empty() {
                    self.session = None;
                }
            }
            let _ = self.events.send(Event::Failed {
                request,
                message: format!("{error:#}"),
            });
        }
    }
    fn apply_command(
        &mut self,
        store: &Store,
        command: Command,
        active: impl Fn(&str) -> bool,
    ) -> Result<()> {
        let (command, recovery) = match command {
            Command::CheckBrowser { request, recovery } => {
                let session = store
                    .browser_recovery_status(recovery)?
                    .context("Browser recovery expired")?;
                ensure!(
                    session.state == "offered",
                    "The browser has not supplied a fresh link"
                );
                let candidate = session.candidate.context("Missing fresh browser link")?;
                (
                    Command::Check {
                        request,
                        job: session.job_id,
                        url: candidate.url,
                    },
                    Some(recovery),
                )
            }
            command => (command, None),
        };
        match command {
            Command::CheckBrowser { .. } => unreachable!("Browser request resolved above"),
            Command::Check { request, job, url } => {
                ensure!(
                    self.session.is_none(),
                    "The previous link check is still open or stopping. Try again."
                );
                ensure!(
                    !active(&job),
                    "Wait for the download to finish pausing before updating its link"
                );
                let job = store.get(&job)?.context("Download no longer exists")?;
                ensure!(
                    matches!(job.status, Status::Paused | Status::Failed),
                    "Only paused or failed downloads can change links"
                );
                ensure!(
                    job.saved_path.is_none(),
                    "This file may already be published. Resume it to recover completion first."
                );
                ensure!(
                    Uuid::parse_str(&job.id).is_ok(),
                    "Invalid saved download identifier"
                );
                let url = validate_url(&url)?.to_string();
                // Checking a link is individual user intent, so queue Start or
                // a due schedule must not silently restart this paused job.
                store.mark_queue_pause(&job.id, false)?;
                let cancel = CancellationToken::new();
                self.session = Some(Session {
                    request,
                    job: job.id.clone(),
                    cancel: cancel.clone(),
                    prepared: None,
                });
                let store = store.clone();
                let events = self.events.clone();
                let limiter = self.limiter.clone();
                self.tasks.spawn(async move {
                    let mut prepared =
                        check(&store, job, url, &cancel, &limiter, |checked, total| {
                            let _ = events.send(Event::Progress {
                                request,
                                checked,
                                total,
                            });
                        })
                        .await?;
                    prepared.recovery = recovery;
                    Ok(prepared)
                });
            }
            Command::Apply { request, restart } => {
                let session = self
                    .session
                    .as_mut()
                    .filter(|s| s.request == request)
                    .context("This link check is no longer available. Check it again.")?;
                ensure!(
                    !session.cancel.is_cancelled(),
                    "This link check was cancelled"
                );
                let prepared = session
                    .prepared
                    .take()
                    .context("Wait for the link check to finish")?;
                let result = prepared.apply(store, restart);
                self.session = None;
                let warning = result?;
                let _ = self.events.send(Event::Applied { request, warning });
            }
            Command::Cancel(request) => {
                if let Some(session) = &self.session
                    && session.request == request
                {
                    session.cancel.cancel();
                    if self.tasks.is_empty() {
                        self.session = None;
                    }
                }
            }
        }
        Ok(())
    }
    pub(crate) async fn shutdown(&mut self) {
        if let Some(session) = &self.session {
            session.cancel.cancel();
        }
        while self.tasks.join_next().await.is_some() {}
        self.session = None;
    }
}
