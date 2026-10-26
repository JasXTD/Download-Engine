//! Scheduled discovery only: workers cannot create downloads. Review commits
//! selected downloads and removes their pending entries in one transaction.
use crate::{
    browser_batch::{Destination, Link},
    grabber_projects::{Project, Settings},
    model::validate_url,
    store::Store,
    website_grabber::{self, Candidate, Config, Report, StopReason},
};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub const MAX_PENDING: usize = 1000;
pub const MAX_HISTORY: usize = 10_000;
// Include source-page metadata so an accumulated preview still fits the
// existing 900,000-byte batch contract when all files are selected.
const MAX_PENDING_BYTES: usize = 700_000;
pub const MAX_INTERVAL_HOURS: u16 = 720;

#[derive(Clone, Debug)]
pub struct Summary {
    pub project: String,
    pub name: String,
    pub enabled: bool,
    pub next_at: Option<i64>,
    pub every_hours: Option<u16>,
    pub running: bool,
    pub last_at: Option<i64>,
    pub message: String,
    pub pending: usize,
}

#[derive(Clone, Debug)]
pub struct Discovery {
    pub id: String,
    pub file: Candidate,
    pub found_at: i64,
}

pub(crate) fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS grabber_schedules (
            project_id TEXT PRIMARY KEY REFERENCES grabber_projects(id) ON DELETE CASCADE,
            enabled INTEGER NOT NULL DEFAULT 0 CHECK(enabled IN (0,1)),
            next_at INTEGER,
            every_hours INTEGER CHECK(every_hours BETWEEN 1 AND 720),
            run_id TEXT, last_at INTEGER, message TEXT NOT NULL DEFAULT '');
         CREATE TABLE IF NOT EXISTS grabber_seen (
            project_id TEXT NOT NULL REFERENCES grabber_projects(id) ON DELETE CASCADE,
            fingerprint BLOB NOT NULL CHECK(length(fingerprint)=32),
            PRIMARY KEY(project_id,fingerprint));
         CREATE TABLE IF NOT EXISTS grabber_pending (
            id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL REFERENCES grabber_projects(id) ON DELETE CASCADE,
            fingerprint BLOB NOT NULL CHECK(length(fingerprint)=32),
            data TEXT NOT NULL, found_at INTEGER NOT NULL,
            UNIQUE(project_id,fingerprint));",
    )?;
    Ok(())
}

fn require_project(conn: &Connection, project: &Project) -> Result<()> {
    ensure!(
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM grabber_projects WHERE id=?1 AND revision=?2)",
            params![project.id, project.revision],
            |r| r.get::<_, bool>(0),
        )?,
        "Project changed or was deleted. Reopen it before changing its schedule."
    );
    Ok(())
}

impl Store {
    pub fn grabber_schedules(&self) -> Result<Vec<Summary>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT p.id,p.name,COALESCE(s.enabled,0),s.next_at,s.every_hours,
             s.run_id IS NOT NULL,s.last_at,COALESCE(s.message,''),
             (SELECT COUNT(*) FROM grabber_pending d WHERE d.project_id=p.id)
             FROM grabber_projects p LEFT JOIN grabber_schedules s ON p.id=s.project_id
             ORDER BY p.name COLLATE NOCASE,p.id",
        )?;
        Ok(stmt
            .query_map([], |r| {
                Ok(Summary {
                    project: r.get(0)?,
                    name: r.get(1)?,
                    enabled: r.get(2)?,
                    next_at: r.get(3)?,
                    every_hours: r.get(4)?,
                    running: r.get(5)?,
                    last_at: r.get(6)?,
                    message: r.get(7)?,
                    pending: r.get::<_, u32>(8)? as usize,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_grabber_schedule(
        &self,
        project: &Project,
        first_at: i64,
        every_hours: Option<u16>,
        now: i64,
    ) -> Result<()> {
        ensure!(
            first_at > now && chrono::DateTime::from_timestamp(first_at, 0).is_some(),
            "Choose a scan time in the future"
        );
        ensure!(
            every_hours.is_none_or(|n| (1..=MAX_INTERVAL_HOURS).contains(&n)),
            "Repeat scans every 1–720 hours"
        );
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_project(&tx, project)?;
        tx.execute(
            "INSERT INTO grabber_schedules(project_id,enabled,next_at,every_hours)
             VALUES(?1,1,?2,?3) ON CONFLICT(project_id) DO UPDATE SET
             enabled=1,next_at=excluded.next_at,every_hours=excluded.every_hours,run_id=NULL",
            params![project.id, first_at, every_hours],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn clear_grabber_schedule(&self, project: &Project) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_project(&tx, project)?;
        tx.execute(
            "UPDATE grabber_schedules SET enabled=0,next_at=NULL,run_id=NULL WHERE project_id=?1",
            [&project.id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn grabber_discoveries(&self, project: &str) -> Result<Vec<Discovery>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare("SELECT id,data,found_at FROM grabber_pending WHERE project_id=?1 ORDER BY found_at,rowid")?;
        let rows = stmt.query_map([project], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get(2)?))
        })?;
        rows.map(|r| {
            let (id, data, found_at) = r?;
            Ok(Discovery {
                id,
                file: serde_json::from_str(&data).context("Invalid saved discovery")?,
                found_at,
            })
        })
        .collect()
    }

    /// Only the selected persisted IDs are removed. Files arriving during review
    /// and unselected files remain pending; repeating a stale submission fails.
    pub fn review_grabber_files(
        &self,
        project: &str,
        ids: &[String],
        add_to: Option<(&str, Destination)>,
    ) -> Result<Vec<String>> {
        ensure!(
            !ids.is_empty() && ids.len() <= MAX_PENDING,
            "Select files to review"
        );
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut links = Vec::new();
        let mut unique = HashSet::new();
        for id in ids {
            ensure!(unique.insert(id), "Repeated preview selection");
            let data: String = tx
                .query_row(
                    "SELECT data FROM grabber_pending WHERE project_id=?1 AND id=?2",
                    params![project, id],
                    |r| r.get(0),
                )
                .optional()?
                .context("Preview changed or was already reviewed. Reopen the preview.")?;
            let file: Candidate = serde_json::from_str(&data)?;
            links.push(Link {
                url: file.url,
                filename: file.filename,
            });
        }
        let jobs = if let Some((queue, destination)) = add_to {
            self.enqueue_browser_batch_in(
                &tx,
                &format!("batch:{}", uuid::Uuid::new_v4()),
                &links,
                None,
                queue,
                destination,
            )?
        } else {
            Vec::new()
        };
        for id in ids {
            tx.execute(
                "DELETE FROM grabber_pending WHERE project_id=?1 AND id=?2",
                params![project, id],
            )?;
        }
        tx.commit()?;
        Ok(jobs)
    }

    /// Explicitly forget reviewed links, keeping pending links deduplicated.
    pub fn reset_grabber_history(&self, project: &Project) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_project(&tx, project)?;
        tx.execute(
            "UPDATE grabber_schedules SET run_id=NULL WHERE project_id=?1",
            [&project.id],
        )?;
        tx.execute(
            "DELETE FROM grabber_seen WHERE project_id=?1 AND fingerprint NOT IN
            (SELECT fingerprint FROM grabber_pending WHERE project_id=?1)",
            [&project.id],
        )?;
        tx.commit()?;
        Ok(())
    }

    // Called by the single desktop scheduler, never by Store::open (which is
    // also used by native browser hosts). A crashed scan is due again at launch.
    pub(crate) fn recover_grabber_scans(&self) -> Result<()> {
        self.connect()?.execute(
            "UPDATE grabber_schedules SET run_id=NULL WHERE run_id IS NOT NULL",
            [],
        )?;
        Ok(())
    }

    fn claim_grabber(&self, now: i64) -> Result<Option<Claim>> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let raw = tx.query_row(
            "SELECT p.id,p.name,p.revision,p.settings,s.next_at,s.every_hours
             FROM grabber_schedules s JOIN grabber_projects p ON p.id=s.project_id
             WHERE s.enabled=1 AND s.next_at<=?1 AND s.run_id IS NULL
             AND (SELECT COUNT(*) FROM grabber_pending d WHERE d.project_id=p.id)<?2
             AND COALESCE((SELECT SUM(length(CAST(data AS BLOB))) FROM grabber_pending d WHERE d.project_id=p.id),0)<?3
             ORDER BY s.next_at,p.id LIMIT 1",
            params![now,MAX_PENDING as i64,MAX_PENDING_BYTES as i64],
            |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?,r.get(4)?,r.get(5)?)),
        ).optional()?;
        let Some((id, name, revision, settings, due, every_hours)) = raw else {
            return Ok(None);
        };
        let settings: Settings = serde_json::from_str(&settings)?;
        let project = Project {
            id,
            name,
            revision,
            settings: settings.validated()?,
        };
        let claim = Claim {
            project,
            due,
            every_hours,
            run: uuid::Uuid::new_v4().to_string(),
        };
        tx.execute(
            "UPDATE grabber_schedules SET run_id=?1 WHERE project_id=?2",
            params![claim.run, claim.project.id],
        )?;
        tx.commit()?;
        Ok(Some(claim))
    }

    fn grabber_claim_current(&self, claim: &Claim) -> Result<bool> {
        claim_current(&self.connect()?, claim)
    }

    fn release_grabber(&self, claim: &Claim) -> Result<()> {
        self.connect()?.execute(
            "UPDATE grabber_schedules SET run_id=NULL WHERE project_id=?1 AND run_id=?2",
            params![claim.project.id, claim.run],
        )?;
        Ok(())
    }

    fn finish_grabber(&self, claim: &Claim, outcome: Result<Report>, now: i64) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !claim_current(&tx, claim)? {
            return Ok(());
        }
        let mut added = 0;
        let mut capacity = false;
        let mut history_full = false;
        let message = match outcome {
            Err(error) => format!("Scan failed: {error:#}"),
            Ok(report) => {
                // Cancellation is not a successful occurrence and must not make
                // unfinished discoveries part of the persistent baseline.
                if report.stop == StopReason::Cancelled {
                    tx.execute(
                        "UPDATE grabber_schedules SET run_id=NULL WHERE project_id=?1",
                        [&claim.project.id],
                    )?;
                    tx.commit()?;
                    return Ok(());
                }
                let (mut pending,mut bytes): (usize,usize) = tx.query_row(
                    "SELECT COUNT(*),COALESCE(SUM(length(CAST(data AS BLOB))),0) FROM grabber_pending WHERE project_id=?1",
                    [&claim.project.id], |r| Ok((r.get::<_,u32>(0)? as usize,r.get::<_,u32>(1)? as usize)))?;
                let mut seen = tx.query_row(
                    "SELECT COUNT(*) FROM grabber_seen WHERE project_id=?1",
                    [&claim.project.id],
                    |r| r.get::<_, u32>(0),
                )? as usize;
                for mut file in report.files {
                    file.url = validate_url(&file.url)?.to_string();
                    validate_url(&file.page)?;
                    let fingerprint = Sha256::digest(file.url.as_bytes());
                    let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM grabber_seen WHERE project_id=?1 AND fingerprint=?2)",
                        params![claim.project.id,&fingerprint[..]], |r| r.get(0))?;
                    if exists {
                        continue;
                    }
                    if seen >= MAX_HISTORY {
                        history_full = true;
                        break;
                    }
                    let data = serde_json::to_string(&file)?;
                    if pending >= MAX_PENDING
                        || bytes.saturating_add(data.len()) > MAX_PENDING_BYTES
                    {
                        capacity = true;
                        break;
                    }
                    tx.execute(
                        "INSERT INTO grabber_seen(project_id,fingerprint) VALUES(?1,?2)",
                        params![claim.project.id, &fingerprint[..]],
                    )?;
                    tx.execute("INSERT INTO grabber_pending(id,project_id,fingerprint,data,found_at) VALUES(?1,?2,?3,?4,?5)",
                        params![uuid::Uuid::new_v4().to_string(),claim.project.id,&fingerprint[..],data,now])?;
                    pending += 1;
                    seen += 1;
                    bytes += data.len();
                    added += 1;
                }
                let mut message = format!(
                    "{} · {added} new files · {} pages read · {} failed",
                    report.stop.label(),
                    report.pages_read,
                    report.pages_failed
                );
                if let Some(issue) = report.issues.first() {
                    // Keep a bounded explanation, not page URLs or response bodies.
                    message.push_str(&format!(
                        " · {}",
                        issue.message.chars().take(240).collect::<String>()
                    ));
                }
                if capacity {
                    message.push_str(" · Preview full; review files before the next scan. Remaining links are not marked as seen.");
                }
                if history_full {
                    message.push_str(" · Schedule stopped at 10,000 remembered links. Reset discovery history before scheduling again.");
                }
                message
            }
        };
        // Skip missed intervals rather than issuing a burst after sleep/restart.
        let next = if history_full {
            None
        } else {
            next_occurrence(claim.due, claim.every_hours, now)?
        };
        tx.execute("UPDATE grabber_schedules SET run_id=NULL,enabled=?1,next_at=?2,last_at=?3,message=?4 WHERE project_id=?5",
            params![next.is_some(),next,now,message.chars().take(1024).collect::<String>(),claim.project.id])?;
        tx.commit()?;
        Ok(())
    }
}

#[derive(Clone)]
struct Claim {
    project: Project,
    run: String,
    due: i64,
    every_hours: Option<u16>,
}

fn claim_current(conn: &Connection, claim: &Claim) -> Result<bool> {
    Ok(conn.query_row("SELECT EXISTS(SELECT 1 FROM grabber_schedules s JOIN grabber_projects p ON p.id=s.project_id
        WHERE p.id=?1 AND p.revision=?2 AND s.run_id=?3 AND s.enabled=1)",
        params![claim.project.id,claim.project.revision,claim.run], |r| r.get(0))?)
}

fn next_occurrence(due: i64, hours: Option<u16>, now: i64) -> Result<Option<i64>> {
    let Some(hours) = hours else { return Ok(None) };
    ensure!(
        (1..=MAX_INTERVAL_HOURS).contains(&hours),
        "Invalid saved scan interval"
    );
    let seconds = i64::from(hours) * 3600;
    let steps = now.saturating_sub(due).max(0) / seconds + 1;
    let next = steps
        .checked_mul(seconds)
        .and_then(|n| due.checked_add(n))
        .context("Scan schedule overflow")?;
    ensure!(
        chrono::DateTime::from_timestamp(next, 0).is_some(),
        "Scan schedule out of range"
    );
    Ok(Some(next))
}

struct Active {
    claim: Claim,
    cancel: CancellationToken,
    task: JoinHandle<Result<Report>>,
}

#[derive(Default)]
pub(crate) struct Controller {
    active: Option<Active>,
}

impl Controller {
    pub(crate) async fn tick(&mut self, store: &Store, now: i64) -> Result<()> {
        if let Some(active) = &self.active {
            if !store.grabber_claim_current(&active.claim)? {
                let active = self.active.take().expect("active scan");
                active.cancel.cancel();
                let _ = active.task.await;
                store.release_grabber(&active.claim)?;
            } else if active.task.is_finished() {
                let active = self.active.take().expect("finished scan");
                let report = active
                    .task
                    .await
                    .context("Scheduled scan worker stopped unexpectedly")
                    .and_then(|r| r);
                if let Err(error) = store.finish_grabber(&active.claim, report, now) {
                    // No immediate request loop when persistence fails. Keep the
                    // occurrence leased for recovery on the next desktop launch.
                    return Err(error.context(
                        "Could not save scheduled scan; restart Download Engine to retry",
                    ));
                }
            }
        }
        if self.active.is_none()
            && let Some(claim) = store.claim_grabber(now)?
        {
            let s = &claim.project.settings;
            let config = match Config::new(&s.url, &s.extensions, s.depth, s.page_limit)
                .and_then(|config| config.with_proxy(store))
            {
                Ok(config) => config,
                Err(error) => {
                    store.finish_grabber(&claim, Err(error), now)?;
                    return Ok(());
                }
            };
            let cancel = CancellationToken::new();
            let token = cancel.clone();
            let task =
                tokio::spawn(async move { website_grabber::scan(config, token, |_| {}).await });
            self.active = Some(Active {
                claim,
                cancel,
                task,
            });
        }
        Ok(())
    }

    pub(crate) async fn shutdown(&mut self, store: &Store) -> Result<()> {
        if let Some(active) = self.active.take() {
            active.cancel.cancel();
            // A completed result should survive a normal exit. Cancellation
            // returns a partial report, which finish_grabber deliberately drops.
            let outcome = active
                .task
                .await
                .context("Scheduled scan worker stopped unexpectedly")
                .and_then(|r| r);
            store.finish_grabber(&active.claim, outcome, chrono::Utc::now().timestamp())?;
            store.release_grabber(&active.claim)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
