//! Idle default-queue activation and explicit single-file starts.
use crate::{
    model::{Job, Status},
    queue::MAIN_QUEUE,
    store::Store,
};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, params};

pub(crate) fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS immediate_downloads(job_id TEXT PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE);")?;
    Ok(())
}

fn idle_main(conn: &Connection, excluding: Option<&str>) -> Result<bool> {
    Ok(conn.query_row("SELECT EXISTS(SELECT 1 FROM download_queues q WHERE q.id='main' AND q.scheduled_at IS NULL
        AND NOT EXISTS(SELECT 1 FROM queue_schedules WHERE queue_id=q.id)
        AND NOT EXISTS(SELECT 1 FROM synchronization_queues WHERE queue_id=q.id)
        AND NOT EXISTS(SELECT 1 FROM job_queues jq JOIN jobs j ON j.id=jq.job_id WHERE jq.queue_id=q.id
          AND (?1 IS NULL OR j.id<>?1) AND (json_extract(j.data,'$.status') IN ('queued','downloading') OR jq.paused_by_queue=1)))",
        [excluding], |row| row.get(0))?)
}

// Run only for a new, committed job or explicit resume, after automation routing.
// Timer ticks must never undo a queue pause merely because no workers are active.
pub(crate) fn wake_idle_main(conn: &Connection, job: &str) -> Result<()> {
    let ready: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM job_queues jq JOIN jobs j ON j.id=jq.job_id
        WHERE jq.job_id=?1 AND jq.queue_id='main' AND json_extract(j.data,'$.status')='queued')",
        [job],
        |row| row.get(0),
    )?;
    if ready && idle_main(conn, Some(job))? {
        conn.execute("UPDATE download_queues SET running=1 WHERE id='main'", [])?;
    }
    Ok(())
}

impl Store {
    pub fn new_download_starts(&self, queue: &str) -> Result<bool> {
        let conn = self.connect()?;
        let running: bool = conn.query_row(
            "SELECT running FROM download_queues WHERE id=?1",
            [queue],
            |row| row.get(0),
        )?;
        Ok(running || (queue == MAIN_QUEUE && idle_main(&conn, None)?))
    }
    pub(crate) fn save_for_start(&self, job: &Job) -> Result<()> {
        job.validate_persisted()?;
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        ensure!(
            tx.execute(
                "UPDATE jobs SET data=?1 WHERE id=?2",
                params![serde_json::to_string(job)?, job.id]
            )? == 1,
            "Download no longer exists"
        );
        // Publishing Queued and waking Main share a commit, so a simultaneous
        // browser handoff cannot see waiting work before the queue is awake.
        wake_idle_main(&tx, &job.id)?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn start_immediately(&self, id: &str) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let raw: String = tx
            .query_row("SELECT data FROM jobs WHERE id=?1", [id], |row| row.get(0))
            .context("Download no longer exists")?;
        let mut job: Job = crate::store::decode_job(id, &raw)?;
        ensure!(
            matches!(job.status, Status::Queued | Status::Paused | Status::Failed),
            "Only a waiting, paused or failed download can start immediately"
        );
        job.status = Status::Queued;
        job.retry = crate::retry::State::default();
        job.quota_waiting = false;
        job.error = None;
        tx.execute(
            "UPDATE jobs SET data=?1 WHERE id=?2",
            params![serde_json::to_string(&job)?, id],
        )?;
        ensure!(
            tx.execute(
                "UPDATE job_queues SET paused_by_queue=0 WHERE job_id=?1",
                [id]
            )? == 1,
            "Download queue no longer exists"
        );
        tx.execute(
            "INSERT OR IGNORE INTO immediate_downloads(job_id) VALUES(?1)",
            [id],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn immediate_downloads(&self) -> Result<Vec<String>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT i.job_id FROM immediate_downloads i JOIN jobs j ON j.id=i.job_id
            WHERE json_extract(j.data,'$.status') IN ('queued','downloading') ORDER BY i.rowid",
        )?;
        Ok(stmt
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub(crate) fn cancel_immediate(&self, job: &str) -> Result<()> {
        self.connect()?
            .execute("DELETE FROM immediate_downloads WHERE job_id=?1", [job])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        model::{Job, Status},
        paths::AppPaths,
        queue::MAIN_QUEUE,
        store::Store,
    };
    #[test]
    fn a_new_download_wakes_an_idle_unscheduled_main_queue() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(AppPaths {
            data: root.path().join("data"),
            downloads: root.path().join("files"),
        })
        .unwrap();
        store.set_queue_running(MAIN_QUEUE, false).unwrap();
        let mut completed = Job::new(
            "https://example.invalid/completed.bin",
            None,
            store.paths.downloads.clone(),
            "completed".into(),
        )
        .unwrap();
        completed.status = Status::Complete;
        store.enqueue(&completed).unwrap();
        assert!(!store.queues().unwrap()[0].running);
        let next = Job::new(
            "https://example.invalid/next.bin",
            None,
            store.paths.downloads.clone(),
            "next".into(),
        )
        .unwrap();
        store.enqueue(&next).unwrap();
        assert!(
            store.queues().unwrap()[0].running,
            "An idle default queue must not strand a newly added download"
        );
        assert_eq!(store.list().unwrap().len(), 2);
    }

    #[test]
    fn auto_start_keeps_custom_schedules_and_existing_waiting_files_held() {
        for mode in [
            "custom",
            "scheduled",
            "recurring",
            "waiting",
            "queue-pausing",
        ] {
            let root = tempfile::tempdir().unwrap();
            let store = Store::open(AppPaths {
                data: root.path().join("data"),
                downloads: root.path().join("files"),
            })
            .unwrap();
            let queue = if mode == "custom" {
                store.create_queue("Later").unwrap().id
            } else {
                MAIN_QUEUE.into()
            };
            let now = chrono::Utc::now().timestamp();
            match mode {
                "scheduled" => store.schedule_queue(&queue, now + 3600, now).unwrap(),
                "recurring" => store
                    .schedule_recurring_queue(
                        &queue,
                        &crate::queue_schedule::RecurringSchedule::new(127, "23:00", "23:01")
                            .unwrap(),
                        now,
                    )
                    .unwrap(),
                "waiting" | "queue-pausing" => {
                    let mut other = Job::new(
                        "https://example.invalid/other.bin",
                        None,
                        store.paths.downloads.clone(),
                        "other".into(),
                    )
                    .unwrap();
                    store.enqueue(&other).unwrap();
                    store.set_queue_running(&queue, false).unwrap();
                    if mode == "queue-pausing" {
                        // A cancelled worker can publish Paused before its queue
                        // ownership is settled; it is still waiting queue work.
                        other.status = Status::Paused;
                        store.save(&other).unwrap();
                        store.mark_queue_pause(&other.id, true).unwrap();
                    }
                }
                _ => {}
            }
            let before = store
                .queues()
                .unwrap()
                .into_iter()
                .find(|q| q.id == queue)
                .unwrap();
            let job = Job::new(
                "https://example.invalid/new.bin",
                None,
                store.paths.downloads.clone(),
                "new".into(),
            )
            .unwrap();
            store.enqueue_batch(&[job], &queue).unwrap();
            assert_eq!(
                store
                    .queues()
                    .unwrap()
                    .into_iter()
                    .find(|q| q.id == queue)
                    .unwrap(),
                before,
                "{mode}"
            );
        }
    }

    #[test]
    fn duplicate_receipts_do_not_restart_a_manually_paused_queue() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(AppPaths {
            data: root.path().join("data"),
            downloads: root.path().join("files"),
        })
        .unwrap();
        let job = Job::new(
            "https://example.invalid/file.bin",
            None,
            store.paths.downloads.clone(),
            "once".into(),
        )
        .unwrap();
        store.set_queue_running(MAIN_QUEUE, false).unwrap();
        store.prepare_capture(&job).unwrap();
        assert!(!store.queues().unwrap()[0].running);
        store.commit_capture(&job.request_id).unwrap();
        assert!(store.queues().unwrap()[0].running);
        store.set_queue_running(MAIN_QUEUE, false).unwrap();
        store.commit_capture(&job.request_id).unwrap();
        store.enqueue(&job).unwrap();
        assert!(!store.queues().unwrap()[0].running);
        assert_eq!(store.list().unwrap().len(), 1);
    }
}
