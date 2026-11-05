//! Completed jobs keep their completed status. Sharing has its own durable
//! opt-in and bounded workers, so it never occupies a download queue slot.
use super::*;
use std::collections::{HashMap, HashSet};
use tokio::task::JoinSet;

const MAX_SEEDS: usize = 4;

impl Store {
    pub fn torrent_sharing(&self, id: &str) -> Result<bool> {
        Ok(self
            .connect()?
            .query_row(
                "SELECT enabled FROM torrent_sharing WHERE job_id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(false))
    }
    pub fn torrent_sharing_error(&self, id: &str) -> Result<Option<String>> {
        Ok(self
            .connect()?
            .query_row(
                "SELECT error FROM torrent_sharing WHERE job_id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }
    pub(crate) fn set_torrent_sharing(&self, id: &str, enabled: bool) -> Result<()> {
        let job = self.get(id)?.context("Download not found")?;
        ensure!(
            job.torrent.is_some() && job.status == Status::Complete,
            "Only completed torrents can be shared"
        );
        if enabled {
            check_allowed(self)?;
            let settings = self.torrent_settings()?;
            ensure!(
                settings.upload && settings.seed_completed,
                "Enable seeding and uploading in Preferences → Torrents first"
            );
            completed_files(self, &job)?;
            ensure!(
                job.integrity.matches() != Some(false),
                "The download checksum does not match"
            );
            ensure!(
                !self
                    .antivirus_report(id)?
                    .is_some_and(|report| report.verdict == crate::antivirus::Verdict::Detected),
                "The antivirus reported a threat; scan the files again before sharing"
            );
        }
        self.connect()?.execute("INSERT INTO torrent_sharing(job_id,enabled,error) VALUES(?1,?2,NULL) ON CONFLICT(job_id) DO UPDATE SET enabled=excluded.enabled,error=NULL", params![id,enabled])?;
        Ok(())
    }
}

pub(crate) fn after_download(store: &Store, id: &str) -> Result<()> {
    let settings = store.torrent_settings()?;
    if settings.enabled
        && settings.upload
        && settings.seed_completed
        && let Some(job) = store.get(id)?
        && job.status == Status::Complete
        && job.torrent.is_some()
    {
        store.connect()?.execute(
            "INSERT OR IGNORE INTO torrent_sharing(job_id,enabled,error) VALUES(?1,1,NULL)",
            [id],
        )?;
    }
    Ok(())
}

#[derive(Default)]
pub(crate) struct Controller {
    tasks: JoinSet<(String, Result<()>)>,
    active: HashMap<String, CancellationToken>,
    owners: HashMap<tokio::task::Id, String>,
    /// A failed database write must never restart something the user just stopped.
    held: HashSet<String>,
    allowed_after_failed_stop_all: Option<HashSet<String>>,
}
impl Controller {
    /// Whether a sharing worker runs for this download.
    pub(crate) fn is_active(&self, id: &str) -> bool {
        self.active.contains_key(id)
    }
    pub(crate) fn allow_start(&mut self, id: &str) {
        self.held.remove(id);
        if let Some(allowed) = &mut self.allowed_after_failed_stop_all {
            allowed.insert(id.to_owned());
        }
    }
    fn complete(
        &mut self,
        store: &Store,
        item: Result<(tokio::task::Id, (String, Result<()>)), tokio::task::JoinError>,
    ) -> Result<()> {
        let (id, result) = match item {
            Ok((task, (id, result))) => {
                self.owners.remove(&task);
                (id, result)
            }
            Err(error) => {
                let id = self
                    .owners
                    .remove(&error.id())
                    .context("Missing seeding worker owner")?;
                // A panic may skip remove/ack. Kill the helper so no orphan can
                // keep uploading or hold files after the worker has gone.
                store.torrents.restart();
                (
                    id,
                    Err(anyhow::anyhow!(
                        "Seeding worker stopped unexpectedly: {error}"
                    )),
                )
            }
        };
        self.active.remove(&id);
        if let Err(error) = result {
            self.held.insert(id.clone());
            store.connect()?.execute(
                "UPDATE torrent_sharing SET enabled=0,error=?1 WHERE job_id=?2",
                params![format!("{error:#}"), id],
            )?;
            self.held.remove(&id);
            return Err(error.context("Torrent sharing stopped"));
        }
        Ok(())
    }
    pub(crate) async fn stop(
        &mut self,
        store: &Store,
        id: Option<&str>,
        remember: bool,
    ) -> Result<()> {
        // Cancellation is independent of persistence, including full disks and SQLite errors.
        for (job, token) in &self.active {
            if id.is_none_or(|id| id == job) {
                token.cancel();
            }
        }
        let mut failure = None;
        if remember {
            let remembered = (|| -> Result<()> {
                match id {
                    Some(id) => {
                        if store.get(id)?.is_some_and(|job| job.torrent.is_some()) {
                            store.connect()?.execute("INSERT INTO torrent_sharing(job_id,enabled,error) VALUES(?1,0,NULL) ON CONFLICT(job_id) DO UPDATE SET enabled=0,error=NULL", [id])?;
                        }
                    }
                    None => {
                        store.connect()?.execute("INSERT INTO torrent_sharing(job_id,enabled,error) SELECT id,0,NULL FROM jobs WHERE json_type(data,'$.torrent')='object' ON CONFLICT(job_id) DO UPDATE SET enabled=0,error=NULL", [])?;
                    }
                };
                Ok(())
            })();
            if let Err(error) = remembered {
                if let Some(id) = id {
                    self.held.insert(id.to_owned());
                } else {
                    self.allowed_after_failed_stop_all = Some(HashSet::new());
                }
                failure = Some(error.context("Sharing is stopped, but its stopped state could not be saved; it may resume after reopening the app"));
            } else if id.is_none() {
                self.allowed_after_failed_stop_all = None;
                self.held.clear();
            } else if let Some(id) = id {
                self.held.remove(id);
            }
        }
        while self.active.keys().any(|job| id.is_none_or(|id| id == job)) {
            let Some(item) = self.tasks.join_next_with_id().await else {
                self.active.clear();
                break;
            };
            if let Err(error) = self.complete(store, item) {
                failure.get_or_insert(error);
            }
        }
        if let Some(error) = failure {
            return Err(error);
        }
        Ok(())
    }
    pub(crate) async fn tick(
        &mut self,
        store: &Store,
        limiter: &Limiter,
        busy: impl Fn(&str) -> bool,
    ) -> Result<()> {
        while let Some(item) = self.tasks.try_join_next_with_id() {
            self.complete(store, item)?;
        }
        let settings = store.torrent_settings()?;
        if !settings.enabled
            || !settings.upload
            || !settings.seed_completed
            || store.proxy_blocks_torrents()?
        {
            return self.stop(store, None, false).await;
        }
        let wanted = {
            let conn = store.connect()?;
            let mut statement =
                conn.prepare("SELECT job_id FROM torrent_sharing WHERE enabled=1 ORDER BY rowid")?;
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let wanted_set: HashSet<&str> = wanted.iter().map(String::as_str).collect();
        self.held.retain(|id| wanted_set.contains(id.as_str()));
        if let Some(allowed) = &mut self.allowed_after_failed_stop_all {
            allowed.retain(|id| wanted_set.contains(id.as_str()));
        }
        for (id, token) in &self.active {
            if !wanted_set.contains(id.as_str()) {
                token.cancel();
            }
        }
        for id in wanted {
            if self.active.len() >= MAX_SEEDS {
                break;
            }
            if self.active.contains_key(&id)
                || self.held.contains(&id)
                || self
                    .allowed_after_failed_stop_all
                    .as_ref()
                    .is_some_and(|allowed| !allowed.contains(&id))
                || busy(&id)
                || crate::file_relocation::is_pending(store, &id)?
            {
                continue;
            }
            let Some(job) = store.get(&id)? else { continue };
            if job.status != Status::Complete || job.torrent.is_none() {
                continue;
            }
            let token = CancellationToken::new();
            self.active.insert(id.clone(), token.clone());
            let (store, limiter) = (store.clone(), limiter.clone());
            let owner = id.clone();
            let worker = self
                .tasks
                .spawn(async move { (id, share(&store, &job, &limiter, &token).await) });
            self.owners.insert(worker.id(), owner);
        }
        Ok(())
    }
}

async fn share(
    store: &Store,
    job: &Job,
    limiter: &Limiter,
    cancel: &CancellationToken,
) -> Result<()> {
    check_allowed(store)?;
    ensure!(
        job.integrity.matches() != Some(false),
        "The download checksum does not match"
    );
    ensure!(
        !store
            .antivirus_report(&job.id)?
            .is_some_and(|report| report.verdict == crate::antivirus::Verdict::Detected),
        "The antivirus reported a threat; scan the files again before sharing"
    );
    let saved = job
        .saved_path
        .as_ref()
        .context("Missing completed torrent")?;
    let files = store.torrent_files(&job.id)?;
    let paths = completed_files(store, job)?;
    let folder = paths.first().is_some_and(|(path, _)| path != saved);
    let output = if folder {
        saved.as_path()
    } else {
        saved.parent().context("Missing download folder")?
    };
    let mut parents = crate::download_file::guard_parents(&output.join(".seed-check"))?;
    let mut seen: std::collections::HashSet<PathBuf> =
        output.ancestors().map(Path::to_owned).collect();
    let mut locked_files = Vec::new();
    // Validate every selected path without touching skipped or unrelated files.
    for (path, size) in &paths {
        if cancel.is_cancelled() {
            return Ok(());
        }
        let parent = path.parent().context("Missing torrent parent folder")?;
        if !seen.contains(parent) {
            for (path, handle) in parent
                .ancestors()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .zip(crate::download_file::guard_parents(path)?)
            {
                if seen.insert(path.to_owned()) {
                    parents.push(handle);
                }
            }
        }
        let file = crate::download_file::read_completed(path)?;
        ensure!(
            file.metadata()?.len() == *size,
            "A completed torrent file changed size; sharing stopped"
        );
        locked_files.push(file);
    }
    let _activity = store.activity.begin(&job.id);
    store.activity.set(&job.id, Phase::CheckingSaved);
    if cancel.is_cancelled() {
        return Ok(());
    }
    let trackers = store.torrent_added_trackers(&job.id)?;
    let lease = store.torrents.lease(store)?;
    let result = async {
    lease
        .call(
            json!({"cmd":"add", "name":job.id, "magnet":job.url, "output":output,
        "only":files.chosen, "seed_only":true,
        "file_name":if folder { None } else { saved.file_name().and_then(|name| name.to_str()) },
        "peers":store.torrents.test_peers(), "trackers":trackers}),
        )
        .await?;
        let mut tick = tokio::time::interval(STATS_EVERY);
        let mut counted = 0u64;
        loop {
            tokio::select! { biased; _ = cancel.cancelled() => return Ok(()), _ = tick.tick() => {} }
            limiter.check_quota().await?;
            let stats = Stats::parse(lease.call(json!({"cmd":"stats","name":job.id})).await?)?;
            if let Some(error) = stats.error { bail!("{error}"); }
            let traffic = stats.uploaded_bytes + stats.downloaded_bytes;
            if traffic > counted { limiter.record_payload(usize::try_from(traffic - counted)?).await?; counted = traffic; }
            store.activity.set(&job.id, if stats.state == "initializing" { Phase::CheckingSaved } else { Phase::Seeding });
            store.activity.set_torrent(&job.id, TorrentSnapshot { peers:stats.peers, download_speed:0,
                upload_speed:stats.upload_bps, uploaded:stats.uploaded_bytes, file_progress:stats.file_progress.into() });
            super::details::serve(store, &lease, &job.id).await?;
        }
    }.await;
    super::details::abandon(store, &job.id);
    let removed = lease.call(json!({"cmd":"remove","name":job.id})).await;
    if removed.is_err() {
        store.torrents.restart();
    }
    result?;
    removed?;
    Ok(())
}
