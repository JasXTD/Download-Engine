use crate::{
    model::{Job, Status},
    queue::{DownloadQueue, MAIN_QUEUE, MAX_ACTIVE, queue_name},
    queue_schedule::RecurringSchedule,
    store::Store,
};
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

#[cfg(test)]
mod tests;

fn legacy_queue_limit(conn: &Connection) -> Result<bool> {
    let schema: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name='download_queues'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(schema
        .as_deref()
        .is_some_and(|sql| sql.contains("BETWEEN 1 AND 8")))
}

pub(crate) fn assign(conn: &Connection, job: &str, queue: &str) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO job_queues(job_id,queue_id,position)
        VALUES (?1,?2,COALESCE((SELECT MAX(position)+1 FROM job_queues WHERE queue_id=?2),0))",
        params![job, queue],
    )?;
    Ok(())
}

pub(crate) fn set_running_in(conn: &Connection, id: &str, running: bool) -> Result<()> {
    let count = conn.execute(
        "UPDATE download_queues SET running=?1,scheduled_at=NULL WHERE id=?2",
        params![running, id],
    )?;
    anyhow::ensure!(count == 1, "Queue no longer exists");
    if !running {
        conn.execute("DELETE FROM immediate_downloads WHERE job_id IN (SELECT job_id FROM job_queues WHERE queue_id=?1)", [id])?;
    }
    // Explicit control suspends recurrence but preserves its configuration.
    conn.execute(
        "UPDATE queue_schedules SET enabled=0 WHERE queue_id=?1",
        [id],
    )?;
    if running {
        prepare_sync_checks(conn, id)?;
    }
    Ok(())
}

impl Store {
    pub(crate) fn initialize_queues(&self) -> Result<()> {
        let mut conn = self.connect()?;
        // The original table has a CHECK <= 8. Rebuild it transactionally using
        // SQLite's table-replacement sequence; never rename the old parent table,
        // which would rewrite child foreign keys. This connection is local to
        // initialization and foreign-key integrity is checked before committing.
        let migrating = legacy_queue_limit(&conn)?;
        if migrating {
            conn.pragma_update(None, "foreign_keys", false)?;
        }
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Another opener may have upgraded while this connection waited for the lock.
        if migrating && legacy_queue_limit(&tx)? {
            tx.execute_batch(
                "CREATE TABLE download_queues_expanded(
                id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE COLLATE NOCASE,
                max_active INTEGER NOT NULL CHECK(max_active BETWEEN 1 AND 32),
                running INTEGER NOT NULL, scheduled_at INTEGER);
                INSERT INTO download_queues_expanded(rowid,id,name,max_active,running,scheduled_at)
                    SELECT rowid,id,name,max_active,running,scheduled_at FROM download_queues;
                DROP TABLE download_queues;
                ALTER TABLE download_queues_expanded RENAME TO download_queues;",
            )?;
        }
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS download_queues(
            id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE COLLATE NOCASE,
            max_active INTEGER NOT NULL CHECK(max_active BETWEEN 1 AND 32),
            running INTEGER NOT NULL, scheduled_at INTEGER);
            CREATE TABLE IF NOT EXISTS job_queues(
            job_id TEXT PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
            queue_id TEXT NOT NULL REFERENCES download_queues(id),
            position INTEGER NOT NULL, paused_by_queue INTEGER NOT NULL DEFAULT 0);
            CREATE INDEX IF NOT EXISTS queue_order ON job_queues(queue_id,position);
            CREATE TABLE IF NOT EXISTS queue_schedules(
            queue_id TEXT PRIMARY KEY REFERENCES download_queues(id) ON DELETE CASCADE,
            days INTEGER NOT NULL CHECK(days BETWEEN 1 AND 127),
            start_minute INTEGER NOT NULL CHECK(start_minute BETWEEN 0 AND 1439),
            stop_minute INTEGER NOT NULL CHECK(stop_minute BETWEEN 0 AND 1439 AND stop_minute<>start_minute),
            enabled INTEGER NOT NULL CHECK(enabled IN (0,1)));
            CREATE TABLE IF NOT EXISTS synchronization_queues(
            queue_id TEXT PRIMARY KEY REFERENCES download_queues(id) ON DELETE CASCADE,
            last_window_start INTEGER);",
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO download_queues(id,name,max_active,running)
            VALUES (?1,'Main queue',3,1)",
            [MAIN_QUEUE],
        )?;
        let missing: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT id FROM jobs WHERE id NOT IN
                (SELECT job_id FROM job_queues) ORDER BY created,rowid",
            )?;
            stmt.query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        for id in missing {
            assign(&tx, &id, MAIN_QUEUE)?;
        }
        if migrating {
            let broken: Option<String> = tx
                .query_row("PRAGMA foreign_key_check", [], |row| row.get(0))
                .optional()?;
            anyhow::ensure!(
                broken.is_none(),
                "Queue migration found an invalid foreign key in {}",
                broken.unwrap_or_default()
            );
        }
        tx.commit()?;
        if migrating {
            conn.pragma_update(None, "foreign_keys", true)?;
        }
        Ok(())
    }

    pub fn queues(&self) -> Result<Vec<DownloadQueue>> {
        read_queues(&self.connect()?)
    }

    pub fn create_queue(&self, name: &str) -> Result<DownloadQueue> {
        self.create_queue_with_mode(name, false)
    }

    pub fn create_sync_queue(&self, name: &str) -> Result<DownloadQueue> {
        self.create_queue_with_mode(name, true)
    }

    fn create_queue_with_mode(&self, name: &str, synchronize: bool) -> Result<DownloadQueue> {
        let name = queue_name(name)?;
        let queue = DownloadQueue {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            max_active: 2,
            running: false,
            scheduled_at: None,
            recurring: None,
            synchronize,
        };
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO download_queues(id,name,max_active,running)
            VALUES (?1,?2,?3,0)",
            params![queue.id, queue.name, queue.max_active as i64],
        )
        .context("Could not create queue; choose a unique name")?;
        if synchronize {
            tx.execute(
                "INSERT INTO synchronization_queues(queue_id) VALUES (?1)",
                [&queue.id],
            )?;
        }
        tx.commit()?;
        Ok(queue)
    }

    pub fn rename_queue(&self, id: &str, name: &str) -> Result<()> {
        let count = self
            .connect()?
            .execute(
                "UPDATE download_queues SET name=?1 WHERE id=?2",
                params![queue_name(name)?, id],
            )
            .context("Choose a unique queue name")?;
        anyhow::ensure!(count == 1, "Queue no longer exists");
        Ok(())
    }

    pub fn delete_queue(&self, id: &str) -> Result<()> {
        anyhow::ensure!(id != MAIN_QUEUE, "The main queue cannot be deleted");
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM job_queues WHERE queue_id=?1",
            [id],
            |r| r.get(0),
        )?;
        anyhow::ensure!(
            count == 0,
            "Move or remove the files before deleting this queue"
        );
        tx.execute("DELETE FROM download_queues WHERE id=?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn queue_jobs(&self, queue: &str) -> Result<Vec<Job>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT j.id,j.data FROM jobs j JOIN job_queues q
            ON q.job_id=j.id WHERE q.queue_id=?1 ORDER BY q.position,j.rowid",
        )?;
        stmt.query_map([queue], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .map(|row| {
            let (id, data) = row?;
            crate::store::decode_job(&id, &data)
        })
        .collect()
    }

    pub fn job_queue(&self, id: &str) -> Result<Option<String>> {
        Ok(self
            .connect()?
            .query_row(
                "SELECT queue_id FROM job_queues WHERE job_id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Membership is separate from worker snapshots, and the entire batch is atomic.
    pub fn enqueue_batch(&self, jobs: &[Job], queue: &str) -> Result<Vec<Job>> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let saved = enqueue_batch_in(&tx, jobs, queue)?;
        tx.commit()?;
        Ok(saved)
    }

    pub fn set_queue_running(&self, id: &str, running: bool) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        set_running_in(&tx, id, running)?;
        tx.commit()?;
        Ok(())
    }

    pub fn set_queue_limit(&self, id: &str, limit: usize) -> Result<()> {
        anyhow::ensure!(
            (1..=MAX_ACTIVE).contains(&limit),
            "Choose between 1 and {MAX_ACTIVE} simultaneous files"
        );
        let count = self.connect()?.execute(
            "UPDATE download_queues SET max_active=?1 WHERE id=?2",
            params![limit as i64, id],
        )?;
        anyhow::ensure!(count == 1, "Queue no longer exists");
        Ok(())
    }

    pub fn schedule_queue(&self, id: &str, at: i64, now: i64) -> Result<()> {
        anyhow::ensure!(at > now, "Choose a time in the future");
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let count = tx.execute(
            "UPDATE download_queues SET running=0,scheduled_at=?1 WHERE id=?2",
            params![at, id],
        )?;
        anyhow::ensure!(count == 1, "Queue no longer exists");
        tx.execute("DELETE FROM queue_schedules WHERE queue_id=?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn schedule_recurring_queue(
        &self,
        id: &str,
        schedule: &RecurringSchedule,
        now: i64,
    ) -> Result<()> {
        schedule.validate()?;
        let running = schedule.enabled && schedule.active_window(now)?.is_some();
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let count = tx.execute(
            "UPDATE download_queues SET running=?1,scheduled_at=NULL WHERE id=?2",
            params![running, id],
        )?;
        anyhow::ensure!(count == 1, "Queue no longer exists");
        tx.execute(
            "INSERT INTO queue_schedules(queue_id,days,start_minute,stop_minute,enabled)
            VALUES (?1,?2,?3,?4,?5) ON CONFLICT(queue_id) DO UPDATE SET
            days=excluded.days,start_minute=excluded.start_minute,stop_minute=excluded.stop_minute,enabled=excluded.enabled",
            params![id, schedule.days, schedule.start_minute, schedule.stop_minute, schedule.enabled],
        )?;
        if running && let Some(window) = schedule.active_window(now)? {
            prepare_sync_window(&tx, id, window.start)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn enable_recurring_schedule(&self, id: &str, now: i64) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let schedule = read_queues(&tx)?
            .into_iter()
            .find(|queue| queue.id == id)
            .context("Queue no longer exists")?
            .recurring
            .context("No recurring schedule is saved")?;
        let running = schedule.active_window(now)?.is_some();
        tx.execute(
            "UPDATE queue_schedules SET enabled=1 WHERE queue_id=?1",
            [id],
        )?;
        tx.execute(
            "UPDATE download_queues SET running=?1,scheduled_at=NULL WHERE id=?2",
            params![running, id],
        )?;
        if let Some(window) = schedule.active_window(now)? {
            prepare_sync_window(&tx, id, window.start)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Remove automation without changing the queue's current run/pause state.
    pub fn clear_schedule(&self, id: &str) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let count = tx.execute(
            "UPDATE download_queues SET scheduled_at=NULL WHERE id=?1",
            [id],
        )?;
        anyhow::ensure!(count == 1, "Queue no longer exists");
        tx.execute("DELETE FROM queue_schedules WHERE queue_id=?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    /// Reconcile with the current clock, rather than replaying missed transitions.
    /// This also stops an overnight queue when the app returns after its window.
    pub fn apply_queue_schedules(&self, now: i64) -> Result<usize> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for queue in read_queues(&tx)? {
            if queue.scheduled_at.is_some_and(|at| at <= now) {
                prepare_sync_checks(&tx, &queue.id)?;
            }
        }
        let mut changed = tx.execute(
            "UPDATE download_queues SET running=1,scheduled_at=NULL
            WHERE scheduled_at IS NOT NULL AND scheduled_at<=?1",
            [now],
        )?;
        for queue in read_queues(&tx)? {
            if let Some(schedule) = queue.recurring.filter(|schedule| schedule.enabled) {
                let window = schedule.active_window(now)?;
                let running = window.is_some();
                if let Some(window) = window {
                    prepare_sync_window(&tx, &queue.id, window.start)?;
                }
                if queue.running != running {
                    changed += tx.execute(
                        "UPDATE download_queues SET running=?1 WHERE id=?2",
                        params![running, queue.id],
                    )?;
                }
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    pub fn move_job(&self, id: &str, queue: &str) -> Result<()> {
        crate::model::validate_job_id(id)?;
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let data: String = tx.query_row("SELECT data FROM jobs WHERE id=?1", [id], |r| r.get(0))?;
        let job = crate::store::decode_job(id, &data)?;
        anyhow::ensure!(
            job.status != Status::Downloading,
            "Pause the download before moving it"
        );
        tx.execute(
            "UPDATE job_queues SET queue_id=?1,
            position=COALESCE((SELECT MAX(position)+1 FROM job_queues WHERE queue_id=?1),0)
            WHERE job_id=?2 AND queue_id<>?1",
            params![queue, id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn reorder_job(&self, id: &str, earlier: bool) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (queue, position): (String, i64) = tx.query_row(
            "SELECT queue_id,position FROM job_queues WHERE job_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let sql = if earlier {
            "SELECT job_id,position FROM job_queues WHERE queue_id=?1 AND position<?2 ORDER BY position DESC LIMIT 1"
        } else {
            "SELECT job_id,position FROM job_queues WHERE queue_id=?1 AND position>?2 ORDER BY position LIMIT 1"
        };
        let neighbor: Option<(String, i64)> = tx
            .query_row(sql, params![queue, position], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        if let Some((other, target)) = neighbor {
            tx.execute(
                "UPDATE job_queues SET position=?1 WHERE job_id=?2",
                params![target, id],
            )?;
            tx.execute(
                "UPDATE job_queues SET position=?1 WHERE job_id=?2",
                params![position, other],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn mark_queue_pause(&self, id: &str, marked: bool) -> Result<()> {
        self.connect()?.execute(
            "UPDATE job_queues SET paused_by_queue=?1 WHERE job_id=?2 AND paused_by_queue<>?1",
            params![marked, id],
        )?;
        Ok(())
    }

    pub(crate) fn queue_paused_jobs(&self) -> Result<Vec<String>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare("SELECT job_id FROM job_queues WHERE paused_by_queue=1")?;
        Ok(stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Call only after the worker has relinquished ownership.
    pub(crate) fn finish_queue_pause(&self, id: &str) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let data: Option<String> = tx
            .query_row(
                "SELECT j.data FROM jobs j JOIN job_queues q
            ON j.id=q.job_id WHERE j.id=?1 AND q.paused_by_queue=1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(data) = data {
            let mut job = crate::store::decode_job(id, &data)?;
            // A quota can require manual restart on a non-resumable server.
            // Automatic queue ownership must not turn that hold into a retry.
            if matches!(job.status, Status::Paused | Status::Downloading) && !job.quota_waiting {
                job.status = Status::Queued;
                job.error = None;
                tx.execute(
                    "UPDATE jobs SET data=?1 WHERE id=?2",
                    params![serde_json::to_string(&job)?, id],
                )?;
            }
            tx.execute(
                "UPDATE job_queues SET paused_by_queue=0 WHERE job_id=?1",
                [id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

fn read_queues(conn: &Connection) -> Result<Vec<DownloadQueue>> {
    let mut stmt = conn.prepare(
        "SELECT q.id,q.name,q.max_active,q.running,q.scheduled_at,
        s.days,s.start_minute,s.stop_minute,s.enabled,x.queue_id IS NOT NULL
        FROM download_queues q LEFT JOIN queue_schedules s ON s.queue_id=q.id
        LEFT JOIN synchronization_queues x ON x.queue_id=q.id
        ORDER BY CASE WHEN q.id='main' THEN 0 ELSE 1 END,q.rowid",
    )?;
    Ok(stmt
        .query_map([], |r| {
            Ok(DownloadQueue {
                id: r.get(0)?,
                name: r.get(1)?,
                max_active: usize::from(r.get::<_, u8>(2)?),
                running: r.get(3)?,
                scheduled_at: r.get(4)?,
                recurring: match r.get::<_, Option<u8>>(5)? {
                    Some(days) => Some(RecurringSchedule {
                        days,
                        start_minute: r.get(6)?,
                        stop_minute: r.get(7)?,
                        enabled: r.get(8)?,
                    }),
                    None => None,
                },
                synchronize: r.get(9)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
}

// The schedule occurrence and all queued checks commit together. Reopening in
// the same window does not trigger another run; a missed window is not replayed.
fn prepare_sync_window(conn: &Connection, queue: &str, start: i64) -> Result<()> {
    let changed = conn.execute(
        "UPDATE synchronization_queues SET last_window_start=?2
         WHERE queue_id=?1 AND (last_window_start IS NULL OR last_window_start<>?2)",
        params![queue, start],
    )?;
    if changed != 0 {
        prepare_sync_checks(conn, queue)?;
    }
    Ok(())
}

fn prepare_sync_checks(conn: &Connection, queue: &str) -> Result<()> {
    let enabled: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM synchronization_queues WHERE queue_id=?1)",
        [queue],
        |row| row.get(0),
    )?;
    if !enabled {
        return Ok(());
    }
    let jobs: Vec<(String, String)> = conn
        .prepare(
            "SELECT j.id,j.data FROM jobs j JOIN job_queues q ON q.job_id=j.id WHERE q.queue_id=?1",
        )?
        .query_map([queue], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, data) in jobs {
        let mut job = crate::store::decode_job(&id, &data)?;
        // Finite HLS jobs keep their reviewed snapshot; a fresh rendition must
        // go through preview instead of a file synchronization request. FTP
        // jobs do not participate in HTTP synchronization checks.
        if job.hls.is_some() || job.ftp.is_some() {
            continue;
        }
        if job.status == Status::Complete {
            crate::synchronization::State::begin(&mut job)?;
        } else if job.status == Status::Failed && job.synchronization.is_some() {
            // Keep a resumable replacement plan and the previous saved copy.
            job.status = Status::Queued;
            job.error = None;
            job.retry = crate::retry::State::default();
            job.synchronization
                .as_mut()
                .expect("Synchronization check")
                .checked_at = None;
        } else {
            // Queued/in-flight checks and individually paused jobs keep their
            // progress and intent. Queue-paused workers use normal recovery.
            continue;
        }
        conn.execute(
            "UPDATE jobs SET data=?1 WHERE id=?2",
            params![serde_json::to_string(&job)?, job.id],
        )?;
    }
    Ok(())
}

/// Shared by desktop batches and atomic browser batch receipts.
pub(crate) fn enqueue_batch_in(conn: &Connection, jobs: &[Job], queue: &str) -> Result<Vec<Job>> {
    anyhow::ensure!(
        !jobs.is_empty() && jobs.len() <= 1000,
        "Add between 1 and 1,000 files"
    );
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM download_queues WHERE id=?1)",
        [queue],
        |r| r.get(0),
    )?;
    anyhow::ensure!(exists, "Queue no longer exists");
    let mut saved = Vec::with_capacity(jobs.len());
    for job in jobs {
        job.validate_persisted()?;
        anyhow::ensure!(
            job.hls.is_some() || !crate::hls::is_playlist_url(&job.url),
            "Use Preview HLS video to choose a quality before adding this stream"
        );
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO jobs(id,request_id,created,data) VALUES(?1,?2,?3,?4)",
            params![
                job.id,
                job.request_id,
                i64::try_from(job.created)?,
                serde_json::to_string(job)?
            ],
        )?;
        let (id, data): (String, String) = conn.query_row(
            "SELECT id,data FROM jobs WHERE request_id=?1",
            [&job.request_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let existing = crate::store::decode_job(&id, &data)?;
        anyhow::ensure!(
            existing.url == job.url,
            "The request identifier belongs to a different URL"
        );
        anyhow::ensure!(
            existing.hls.as_ref().map(|v| &v.playlist) == job.hls.as_ref().map(|v| &v.playlist),
            "The request identifier belongs to another HLS preview"
        );
        anyhow::ensure!(
            existing.hls.as_ref().and_then(|v| v.youtube.as_ref())
                == job.hls.as_ref().and_then(|v| v.youtube.as_ref()),
            "The request identifier belongs to another YouTube selection"
        );
        anyhow::ensure!(
            existing.hls.as_ref().and_then(|v| v.bilibili.as_ref())
                == job.hls.as_ref().and_then(|v| v.bilibili.as_ref()),
            "The request identifier belongs to another Bilibili selection"
        );
        anyhow::ensure!(
            existing.hls.as_ref().and_then(|v| v.tiktok.as_ref())
                == job.hls.as_ref().and_then(|v| v.tiktok.as_ref()),
            "The request identifier belongs to another TikTok selection"
        );
        anyhow::ensure!(
            existing.ftp.as_ref().map(|v| v.security) == job.ftp.as_ref().map(|v| v.security),
            "The request identifier belongs to a different FTP security mode"
        );
        assign(conn, &existing.id, queue)?;
        if inserted == 1 {
            crate::automation::on_added(conn, &existing)?;
            crate::queue_start::wake_idle_main(conn, &existing.id)?;
        }
        saved.push(existing);
    }
    Ok(saved)
}
