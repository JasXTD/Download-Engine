//! Completed jobs keep their completed status. Sharing has its own durable
//! opt-in and bounded workers, so it never occupies a download queue slot.
use super::limits::{self, Limits, Reached, Shared};
use super::*;
use crate::transfer_activity::SharingView;
use std::collections::{HashMap, HashSet};
use std::num::NonZeroU64;
use tokio::task::JoinSet;

const MAX_SEEDS: usize = 4;
/// Totals reach the database at least this often while sharing, and at every stop.
const SAVE_TOTALS_EVERY: Duration = Duration::from_secs(60);
/// How often a running share looks for changed limits in Preferences.
const READ_LIMITS_EVERY: Duration = Duration::from_secs(5);

/// What one torrent has shared so far, added up across every run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SharingTotals {
    pub uploaded: u64,
    pub shared_secs: u64,
    /// Which limit stopped sharing, until it starts again.
    pub stopped: Option<Reached>,
    /// Set by sharing again after a limit stopped it: then only a person stops it.
    pub unlimited: bool,
}

fn as_count(value: u64) -> Result<i64> {
    i64::try_from(value).context("The sharing totals are too large to save")
}

fn save_totals(
    conn: &rusqlite::Connection,
    id: &str,
    uploaded: u64,
    shared_secs: u64,
    stopped: Option<Reached>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO torrent_sharing_totals(job_id,uploaded,shared_secs,stopped) VALUES(?1,?2,?3,?4)
         ON CONFLICT(job_id) DO UPDATE SET uploaded=excluded.uploaded,
         shared_secs=excluded.shared_secs, stopped=excluded.stopped",
        params![
            id,
            as_count(uploaded)?,
            as_count(shared_secs)?,
            stopped.map(Reached::as_str)
        ],
    )?;
    Ok(())
}

/// Saves the totals and stops sharing in one step, so a limit can't be reached twice or
/// leave sharing switched on.
fn stop_at_limit(store: &Store, id: &str, shared: Shared, reached: Reached) -> Result<()> {
    let mut conn = store.connect()?;
    let tx = conn.transaction()?;
    save_totals(&tx, id, shared.uploaded, shared.shared_secs, Some(reached))?;
    tx.execute(
        "UPDATE torrent_sharing SET enabled=0,error=NULL WHERE job_id=?1",
        [id],
    )?;
    tx.commit()?;
    Ok(())
}

impl Store {
    pub fn torrent_sharing_totals(&self, id: &str) -> Result<SharingTotals> {
        let row = self
            .connect()?
            .query_row(
                "SELECT uploaded,shared_secs,stopped,unlimited FROM torrent_sharing_totals WHERE job_id=?1",
                [id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, bool>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((uploaded, shared_secs, stopped, unlimited)) = row else {
            return Ok(SharingTotals::default());
        };
        let invalid = || anyhow::anyhow!("Invalid saved sharing totals");
        Ok(SharingTotals {
            uploaded: u64::try_from(uploaded).map_err(|_| invalid())?,
            shared_secs: u64::try_from(shared_secs).map_err(|_| invalid())?,
            stopped: stopped
                .map(|text| Reached::parse(&text).ok_or_else(invalid))
                .transpose()?,
            unlimited,
        })
    }

    /// Uploads while downloading count toward the share ratio too.
    pub(crate) fn add_torrent_uploaded(&self, id: &str, bytes: u64) -> Result<()> {
        if bytes == 0 {
            return Ok(());
        }
        self.connect()?.execute(
            "INSERT INTO torrent_sharing_totals(job_id,uploaded,shared_secs) VALUES(?1,?2,0)
             ON CONFLICT(job_id) DO UPDATE SET uploaded=uploaded+excluded.uploaded",
            params![id, as_count(bytes)?],
        )?;
        Ok(())
    }

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
        let mut conn = self.connect()?;
        let tx = conn.transaction()?;
        tx.execute("INSERT INTO torrent_sharing(job_id,enabled,error) VALUES(?1,?2,NULL) ON CONFLICT(job_id) DO UPDATE SET enabled=excluded.enabled,error=NULL", params![id,enabled])?;
        if enabled {
            // Sharing again after a limit stopped it is a person's choice to keep going.
            tx.execute(
                "UPDATE torrent_sharing_totals SET stopped=NULL,unlimited=1 WHERE job_id=?1 AND stopped IS NOT NULL",
                [id],
            )?;
        }
        tx.commit()?;
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
    let chosen_size =
        NonZeroU64::new(files.chosen_size()).context("This torrent has nothing to share")?;
    let totals = store.torrent_sharing_totals(&job.id)?;
    let read_limits = || -> Result<Limits> {
        Ok(if totals.unlimited {
            Limits::default()
        } else {
            store.torrent_settings()?.sharing_limits()
        })
    };
    let mut limits = read_limits()?;
    let before = Shared {
        uploaded: totals.uploaded,
        chosen_size,
        shared_secs: totals.shared_secs,
    };
    // A torrent can reach its ratio while it downloads. Then sharing never starts.
    if let Some(reached) = limits::reached(before, limits) {
        return stop_at_limit(store, &job.id, before, reached);
    }
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
    // What this run adds to the saved totals. Time only counts once the engine shares.
    let mut uploaded_now = 0u64;
    let mut started: Option<Instant> = None;
    let mut limit_stopped = false;
    let now = |uploaded_now: u64, started: Option<Instant>| Shared {
        uploaded: totals.uploaded.saturating_add(uploaded_now),
        chosen_size,
        shared_secs: totals
            .shared_secs
            .saturating_add(started.map_or(0, |at| at.elapsed().as_secs())),
    };
    let result = async {
    lease
        .call(
            json!({"cmd":"add", "name":job.id, "magnet":job.url, "output":output,
        "only":files.chosen, "seed_only":true,
        "file_name":if folder { None } else { saved.file_name().and_then(|name| name.to_str()) },
        "peers":store.torrents.test_peers(), "trackers":trackers}),
        )
        .await?;
        started = Some(Instant::now());
        let mut tick = tokio::time::interval(STATS_EVERY);
        let mut counted = 0u64;
        let (mut saved_at, mut limits_at) = (Instant::now(), Instant::now());
        loop {
            tokio::select! { biased; _ = cancel.cancelled() => return Ok(()), _ = tick.tick() => {} }
            limiter.check_quota().await?;
            let stats = Stats::parse(lease.call(json!({"cmd":"stats","name":job.id})).await?)?;
            if let Some(error) = stats.error { bail!("{error}"); }
            store.torrents.set_router(stats.router);
            let traffic = stats.uploaded_bytes + stats.downloaded_bytes;
            if traffic > counted { limiter.record_payload(usize::try_from(traffic - counted)?).await?; counted = traffic; }
            uploaded_now = uploaded_now.max(stats.uploaded_bytes);
            if limits_at.elapsed() >= READ_LIMITS_EVERY {
                limits = read_limits()?;
                limits_at = Instant::now();
            }
            let shared = now(uploaded_now, started);
            if let Some(reached) = limits::reached(shared, limits) {
                stop_at_limit(store, &job.id, shared, reached)?;
                limit_stopped = true;
                return Ok(());
            }
            if saved_at.elapsed() >= SAVE_TOTALS_EVERY {
                save_totals(&store.connect()?, &job.id, shared.uploaded, shared.shared_secs, None)?;
                saved_at = Instant::now();
            }
            store.activity.set(&job.id, if stats.state == "initializing" { Phase::CheckingSaved } else { Phase::Seeding });
            store.activity.set_torrent(&job.id, TorrentSnapshot { peers:stats.peers, download_speed:0,
                upload_speed:stats.upload_bps, uploaded:shared.uploaded, file_progress:stats.file_progress.into(),
                sharing: Some(SharingView {
                    ratio_hundredths: limits::ratio_hundredths(shared.uploaded, chosen_size),
                    shared_secs: shared.shared_secs,
                    limit_ratio: limits.ratio_hundredths.map(|ratio| ratio.get()),
                    limit_seconds: limits.seconds.map(|seconds| seconds.get()),
                }) });
            super::details::serve(store, &lease, &job.id).await?;
        }
    }.await;
    super::details::abandon(store, &job.id);
    // Whatever ended this run, what it shared is kept. If that can't be saved, sharing
    // stops with an error rather than running on without its count.
    let kept = if limit_stopped || started.is_none() {
        Ok(())
    } else {
        let shared = now(uploaded_now, started);
        store.connect().and_then(|conn| {
            save_totals(&conn, &job.id, shared.uploaded, shared.shared_secs, None)
        })
    };
    let removed = lease.call(json!({"cmd":"remove","name":job.id})).await;
    if removed.is_err() {
        store.torrents.restart();
    }
    result?;
    kept?;
    removed?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enable_sharing(store: &Store) {
        store
            .save_torrent_settings(Settings {
                enabled: true,
                dht: false,
                seed_completed: true,
                share_ratio: Some(100),
                ..Default::default()
            })
            .unwrap();
    }

    #[tokio::test]
    async fn a_torrent_already_past_its_ratio_stops_without_starting_the_helper() {
        let (_temp, store, job) = completed_folder_fixture();
        enable_sharing(&store);
        store.set_torrent_sharing(&job.id, true).unwrap();
        // 16 bytes are chosen, and 16 went out while downloading: ratio 1.
        store.add_torrent_uploaded(&job.id, 16).unwrap();
        let limiter = Limiter::default();
        let mut controller = Controller::default();
        controller.tick(&store, &limiter, |_| false).await.unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while controller.is_active(&job.id) {
            assert!(std::time::Instant::now() < deadline, "the worker never ended");
            tokio::time::sleep(Duration::from_millis(20)).await;
            controller.tick(&store, &limiter, |_| false).await.unwrap();
        }
        assert!(!store.torrent_sharing(&job.id).unwrap());
        assert_eq!(store.torrent_sharing_error(&job.id).unwrap(), None);
        assert_eq!(
            store.torrent_sharing_totals(&job.id).unwrap(),
            SharingTotals {
                uploaded: 16,
                shared_secs: 0,
                stopped: Some(Reached::Ratio),
                unlimited: false,
            }
        );
        assert!(!store.torrents.is_running(), "nothing was started to find that out");
    }

    #[test]
    fn sharing_again_after_a_limit_lifts_the_limit_for_that_torrent_only() {
        let (_temp, store, job) = completed_folder_fixture();
        enable_sharing(&store);
        store.add_torrent_uploaded(&job.id, 40).unwrap();
        stop_at_limit(
            &store,
            &job.id,
            Shared {
                uploaded: 40,
                chosen_size: NonZeroU64::new(16).unwrap(),
                shared_secs: 5,
            },
            Reached::Ratio,
        )
        .unwrap();
        store.set_torrent_sharing(&job.id, false).unwrap();
        assert_eq!(
            store.torrent_sharing_totals(&job.id).unwrap().stopped,
            Some(Reached::Ratio),
            "stopping by hand keeps the reason"
        );
        store.set_torrent_sharing(&job.id, true).unwrap();
        let totals = store.torrent_sharing_totals(&job.id).unwrap();
        assert_eq!(totals.stopped, None);
        assert!(totals.unlimited);
        assert_eq!((totals.uploaded, totals.shared_secs), (40, 5));
        // Ending and starting again by hand keeps the choice; nothing resets the totals.
        store.set_torrent_sharing(&job.id, false).unwrap();
        store.set_torrent_sharing(&job.id, true).unwrap();
        assert!(store.torrent_sharing_totals(&job.id).unwrap().unlimited);
    }

    #[test]
    fn totals_add_up_and_a_damaged_row_is_refused() {
        let (_temp, store, job) = completed_folder_fixture();
        assert_eq!(
            store.torrent_sharing_totals(&job.id).unwrap(),
            SharingTotals::default()
        );
        store.add_torrent_uploaded(&job.id, 7).unwrap();
        store.add_torrent_uploaded(&job.id, 0).unwrap();
        store.add_torrent_uploaded(&job.id, 5).unwrap();
        assert_eq!(store.torrent_sharing_totals(&job.id).unwrap().uploaded, 12);
        assert!(store.add_torrent_uploaded(&job.id, u64::MAX).is_err());
        let conn = store.connect().unwrap();
        assert!(
            conn.execute(
                "UPDATE torrent_sharing_totals SET uploaded=-1 WHERE job_id=?1",
                [&job.id],
            )
            .is_err(),
            "the database refuses negative totals"
        );
        assert!(
            conn.execute(
                "UPDATE torrent_sharing_totals SET stopped='forever' WHERE job_id=?1",
                [&job.id],
            )
            .is_err()
        );
        assert_eq!(store.torrent_sharing_totals(&job.id).unwrap().uploaded, 12);
    }
}
