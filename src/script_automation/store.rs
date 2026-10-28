use super::*;
use crate::store::Store;
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::json;

pub(crate) fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS automation_scripts(
          id TEXT PRIMARY KEY,data TEXT NOT NULL,cursor INTEGER NOT NULL,next_at INTEGER);
         CREATE TABLE IF NOT EXISTS script_runs(
          id TEXT PRIMARY KEY,script_id TEXT NOT NULL,data TEXT NOT NULL,body TEXT NOT NULL,
          timeout INTEGER NOT NULL,state TEXT NOT NULL,stop_requested INTEGER NOT NULL DEFAULT 0,
          log_bytes INTEGER NOT NULL DEFAULT 0,truncated INTEGER NOT NULL DEFAULT 0);
         CREATE INDEX IF NOT EXISTS script_run_state ON script_runs(state);
         CREATE TABLE IF NOT EXISTS script_logs(
          seq INTEGER PRIMARY KEY AUTOINCREMENT,run_id TEXT NOT NULL REFERENCES script_runs(id) ON DELETE CASCADE,
          at INTEGER NOT NULL,level TEXT NOT NULL,text TEXT NOT NULL);
         CREATE INDEX IF NOT EXISTS script_log_run ON script_logs(run_id,seq);
         CREATE TABLE IF NOT EXISTS script_results(
          run_id TEXT PRIMARY KEY REFERENCES script_runs(id) ON DELETE CASCADE,data TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS script_events(
          seq INTEGER PRIMARY KEY AUTOINCREMENT,job_id TEXT NOT NULL,queue_id TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS script_retries(job_id TEXT PRIMARY KEY,count INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS script_state(
          script_id TEXT NOT NULL,key TEXT NOT NULL,value TEXT NOT NULL,updated INTEGER NOT NULL,
          PRIMARY KEY(script_id,key));",
    )?;
    // Older profiles have completion events only; their rows stay completions.
    crate::store::add_column(
        conn,
        "script_events",
        "kind",
        "TEXT NOT NULL DEFAULT 'completed'",
    )?;
    crate::store::add_column(conn, "script_runs", "notices", "INTEGER NOT NULL DEFAULT 0")?;
    conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS script_download_completed AFTER UPDATE OF data ON jobs
         WHEN json_extract(NEW.data,'$.status')='complete' AND json_extract(OLD.data,'$.status')<>'complete'
         BEGIN
          INSERT INTO script_events(job_id,queue_id) VALUES(NEW.id,COALESCE((SELECT queue_id FROM job_queues WHERE job_id=NEW.id),'main'));
          DELETE FROM script_events WHERE seq < (SELECT MAX(seq)-9999 FROM script_events);
         END;
         -- Only a final failure: automatic retries keep a download queued.
         CREATE TRIGGER IF NOT EXISTS script_download_failed AFTER UPDATE OF data ON jobs
         WHEN json_extract(NEW.data,'$.status')='failed' AND json_extract(OLD.data,'$.status')<>'failed'
         BEGIN
          INSERT INTO script_events(job_id,queue_id,kind) VALUES(NEW.id,COALESCE((SELECT queue_id FROM job_queues WHERE job_id=NEW.id),'main'),'failed');
          DELETE FROM script_events WHERE seq < (SELECT MAX(seq)-9999 FROM script_events);
         END;
         -- Scripts may retry a download again once it has finished.
         CREATE TRIGGER IF NOT EXISTS script_retries_complete AFTER UPDATE OF data ON jobs
         WHEN json_extract(NEW.data,'$.status')='complete' AND json_extract(OLD.data,'$.status')<>'complete'
         BEGIN DELETE FROM script_retries WHERE job_id=NEW.id; END;
         CREATE TRIGGER IF NOT EXISTS script_retries_removed AFTER DELETE ON jobs
         BEGIN DELETE FROM script_retries WHERE job_id=OLD.id; END;",
    )?;
    Ok(())
}

/// What a script receives for one download in `event.downloads`. Sample runs
/// use the same shape. The address is left out: it can contain private tokens.
fn event_download(
    kind: &str,
    id: &str,
    queue: &str,
    job: Option<&crate::model::Job>,
) -> serde_json::Value {
    let name = job.map(|j| &j.name);
    if kind == "failed" {
        json!({"id":id,"queue":queue,"name":name,"error":job.and_then(|j| j.error.as_ref()),
            "attempts":job.map(|j| u32::from(j.retry.attempts) + 1)})
    } else {
        json!({"id":id,"queue":queue,"name":name,"saved_path":job.and_then(|j| j.saved_path.as_ref())})
    }
}

fn latest(conn: &Connection) -> Result<i64> {
    Ok(
        conn.query_row("SELECT COALESCE(MAX(seq),0) FROM script_events", [], |r| {
            r.get(0)
        })?,
    )
}
fn definition(conn: &Connection, id: &str) -> Result<Script> {
    let data: String = conn
        .query_row(
            "SELECT data FROM automation_scripts WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?
        .context("Script no longer exists")?;
    Ok(serde_json::from_str(&data)?)
}
fn definitions(conn: &Connection) -> Result<Vec<Script>> {
    let mut stmt = conn.prepare("SELECT data FROM automation_scripts ORDER BY rowid")?;
    let data = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    data.into_iter()
        .map(|v| Ok(serde_json::from_str(&v)?))
        .collect()
}
fn busy(conn: &Connection, id: &str) -> Result<bool> {
    Ok(conn.query_row("SELECT EXISTS(SELECT 1 FROM script_runs WHERE script_id=?1 AND state IN ('queued','running'))", [id], |r| r.get(0))?)
}
fn update_run(conn: &Connection, run: &Run) -> Result<()> {
    conn.execute(
        "UPDATE script_runs SET data=?1,state=?2 WHERE id=?3",
        params![serde_json::to_string(run)?, run.state, run.id],
    )?;
    Ok(())
}
fn read_run(conn: &Connection, id: &str) -> Result<Run> {
    let data: String = conn
        .query_row("SELECT data FROM script_runs WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()?
        .context("Run no longer exists")?;
    Ok(serde_json::from_str(&data)?)
}
fn prune_runs(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM script_runs WHERE state NOT IN ('queued','running') AND rowid NOT IN (SELECT rowid FROM script_runs WHERE state NOT IN ('queued','running') ORDER BY rowid DESC LIMIT 100)", [])?;
    Ok(())
}
fn enqueue(
    conn: &Connection,
    script: &Script,
    id: &str,
    context: serde_json::Value,
    now: i64,
) -> Result<Run> {
    ensure!(
        !busy(conn, &script.id)?,
        "This script already has a queued or running instance"
    );
    let run = Run {
        id: id.into(),
        script_id: script.id.clone(),
        name: script.name.clone(),
        revision: script.revision,
        language: script.language,
        trigger: context["trigger"].as_str().unwrap_or("manual").into(),
        context,
        queued_at: now,
        started_at: None,
        finished_at: None,
        state: "queued".into(),
        error: None,
        exit_code: None,
    };
    conn.execute("INSERT INTO script_runs(id,script_id,data,body,timeout,state) VALUES(?1,?2,?3,?4,?5,'queued')",
        params![id,script.id,serde_json::to_string(&run)?,script.body,script.timeout_seconds])?;
    prune_runs(conn)?;
    Ok(run)
}

impl Store {
    pub fn scripts(&self) -> Result<Vec<Script>> {
        definitions(&self.connect()?)
    }
    pub fn script(&self, id: &str) -> Result<Script> {
        definition(&self.connect()?, id)
    }

    pub fn save_script(&self, script: &Script) -> Result<Script> {
        script.validate()?;
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old: Option<String> = tx
            .query_row(
                "SELECT data FROM automation_scripts WHERE id=?1",
                [&script.id],
                |r| r.get(0),
            )
            .optional()?;
        let revision = old
            .map(|v| serde_json::from_str::<Script>(&v))
            .transpose()?
            .map_or(0, |s| s.revision);
        ensure!(
            revision == script.revision,
            "This script changed elsewhere. Reload it before saving"
        );
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM automation_scripts WHERE id<>?1",
            [&script.id],
            |r| r.get(0),
        )?;
        ensure!(count < 50, "Keep at most 50 scripts");
        let now = chrono::Utc::now().timestamp();
        let next_at = match &script.trigger {
            Trigger::Interval { minutes } => Some(now + i64::from(*minutes) * 60),
            _ => None,
        };
        let queue = match &script.trigger {
            Trigger::DownloadCompleted { queue } | Trigger::DownloadFailed { queue } => {
                queue.as_ref()
            }
            Trigger::QueueCompleted { queue } => Some(queue),
            _ => None,
        };
        if let Some(queue) = queue {
            ensure!(
                tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM download_queues WHERE id=?1)",
                    [queue],
                    |r| r.get::<_, bool>(0)
                )?,
                "Choose an existing queue"
            );
        }
        let mut saved = script.clone();
        saved.name = saved.name.trim().into();
        saved.revision = revision
            .checked_add(1)
            .context("Script revision overflow")?;
        tx.execute("INSERT INTO automation_scripts(id,data,cursor,next_at) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET data=excluded.data,cursor=excluded.cursor,next_at=excluded.next_at",
            params![saved.id,serde_json::to_string(&saved)?,latest(&tx)?,next_at])?;
        tx.commit()?;
        Ok(saved)
    }
    pub fn remove_script(&self, id: &str) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure!(!busy(&tx, id)?, "Stop this script before removing it");
        ensure!(
            tx.execute("DELETE FROM automation_scripts WHERE id=?1", [id])? == 1,
            "Script no longer exists"
        );
        tx.execute("DELETE FROM script_state WHERE script_id=?1", [id])?;
        tx.commit()?;
        Ok(())
    }
    pub fn run_script(&self, id: &str, request: &str) -> Result<Run> {
        self.run_script_revision(id, request, None)
    }
    pub(crate) fn run_mcp_script(&self, id: &str, revision: u64, request: &str) -> Result<Run> {
        self.run_script_revision(id, request, Some(revision))
    }
    fn run_script_revision(&self, id: &str, request: &str, approval: Option<u64>) -> Result<Run> {
        crate::model::validate_job_id(request)?;
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM script_runs WHERE id=?1)",
            [request],
            |r| r.get::<_, bool>(0),
        )? {
            let run = read_run(&tx, request)?;
            ensure!(run.script_id == id, "Run ID belongs to another script");
            return Ok(run);
        }
        let script = definition(&tx, id)?;
        if let Some(revision) = approval {
            crate::mcp::access::check_approval(&tx, &script, revision)?;
        }
        // A reader never runs with automation powers; it only reads pages.
        ensure!(
            !matches!(script.trigger, Trigger::SiteReader { .. }),
            "Site readers run when a matching page is opened. Use Test on URL to try one."
        );
        let run = enqueue(
            &tx,
            &script,
            request,
            json!({"trigger":"manual","downloads":[]}),
            chrono::Utc::now().timestamp(),
        )?;
        tx.commit()?;
        Ok(run)
    }
    /// Run an event script now with downloads the person picked, in the event
    /// shape its trigger delivers, marked `sample`. A Queue completed script
    /// takes no downloads: its event names only the queue.
    pub fn run_script_sample(&self, id: &str, request: &str, downloads: &[String]) -> Result<Run> {
        crate::model::validate_job_id(request)?;
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM script_runs WHERE id=?1)",
            [request],
            |r| r.get::<_, bool>(0),
        )? {
            let run = read_run(&tx, request)?;
            ensure!(run.script_id == id, "Run ID belongs to another script");
            return Ok(run);
        }
        let script = definition(&tx, id)?;
        let (name, kind, queue) = match &script.trigger {
            Trigger::DownloadCompleted { queue } => {
                ("download_completed", Some("completed"), queue.clone())
            }
            Trigger::DownloadFailed { queue } => ("download_failed", Some("failed"), queue.clone()),
            Trigger::QueueCompleted { queue } => ("queue_completed", None, Some(queue.clone())),
            _ => anyhow::bail!(
                "Only Download completed, Download failed and Queue completed scripts run with a sample"
            ),
        };
        let mut entries = Vec::new();
        match kind {
            None => ensure!(
                downloads.is_empty(),
                "A Queue completed event lists no downloads"
            ),
            Some(kind) => {
                ensure!(
                    (1..=100).contains(&downloads.len()),
                    "Choose 1 to 100 downloads for the sample"
                );
                let wanted = if kind == "failed" {
                    crate::model::Status::Failed
                } else {
                    crate::model::Status::Complete
                };
                for (index, job) in downloads.iter().enumerate() {
                    ensure!(
                        !downloads[..index].contains(job),
                        "Each download can be chosen once"
                    );
                    let data: String = tx
                        .query_row("SELECT data FROM jobs WHERE id=?1", [job], |r| r.get(0))
                        .optional()?
                        .context("A chosen download no longer exists")?;
                    let job: crate::model::Job = serde_json::from_str(&data)?;
                    ensure!(
                        job.status == wanted,
                        "{} is not {}",
                        job.name,
                        if kind == "failed" {
                            "in Failed"
                        } else {
                            "finished"
                        }
                    );
                    let queue: String = tx
                        .query_row(
                            "SELECT queue_id FROM job_queues WHERE job_id=?1",
                            [&job.id],
                            |r| r.get(0),
                        )
                        .optional()?
                        .unwrap_or_else(|| crate::queue::MAIN_QUEUE.into());
                    entries.push(event_download(kind, &job.id, &queue, Some(&job)));
                }
            }
        }
        let run = enqueue(
            &tx,
            &script,
            request,
            json!({"trigger":name,"queue":queue,"downloads":entries,"events_expired":false,"sample":true}),
            chrono::Utc::now().timestamp(),
        )?;
        tx.commit()?;
        Ok(run)
    }
    /// Downloads a sample run of `script` can use, newest first: finished ones
    /// for Download completed, ones that need attention for Download failed.
    pub fn sample_candidates(&self, script: &Script) -> Result<Vec<SampleCandidate>> {
        let (status, queue) = match &script.trigger {
            Trigger::DownloadCompleted { queue } => ("complete", queue.as_deref()),
            Trigger::DownloadFailed { queue } => ("failed", queue.as_deref()),
            _ => return Ok(Vec::new()),
        };
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT j.data,COALESCE(q.queue_id,'main') FROM jobs j LEFT JOIN job_queues q ON q.job_id=j.id
             WHERE json_extract(j.data,'$.status')=?1 AND (?2 IS NULL OR q.queue_id=?2)
             ORDER BY j.created DESC,j.rowid DESC LIMIT 200",
        )?;
        let rows = stmt
            .query_map(params![status, queue], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(data, queue)| {
                let job: crate::model::Job = serde_json::from_str(&data)?;
                Ok(SampleCandidate {
                    attempts: u32::from(job.retry.attempts) + 1,
                    id: job.id,
                    name: job.name,
                    queue,
                    error: job.error,
                    created: job.created,
                })
            })
            .collect()
    }
    /// The script a worker's run belongs to, and whether that run is a sample.
    pub fn script_for_run(&self, run: &str) -> Result<(Script, bool)> {
        let conn = self.connect()?;
        let run = read_run(&conn, run)?;
        let sample = run.context["sample"].as_bool() == Some(true);
        Ok((definition(&conn, &run.script_id)?, sample))
    }
    /// What a script keeps between runs, by key.
    pub fn script_values(&self, script: &str) -> Result<Vec<StoredValue>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT key,value,updated FROM script_state WHERE script_id=?1 ORDER BY key",
        )?;
        let rows = stmt
            .query_map([script], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get(2)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(key, value, updated)| {
                Ok(StoredValue {
                    key,
                    value: serde_json::from_str(&value)?,
                    updated,
                })
            })
            .collect()
    }
    pub fn script_value(&self, script: &str, key: &str) -> Result<Option<serde_json::Value>> {
        let value: Option<String> = self
            .connect()?
            .query_row(
                "SELECT value FROM script_state WHERE script_id=?1 AND key=?2",
                params![script, key],
                |r| r.get(0),
            )
            .optional()?;
        Ok(value.map(|v| serde_json::from_str(&v)).transpose()?)
    }
    /// Keep `value` under `key` for the next run; null removes the key.
    pub fn set_script_value(
        &self,
        script: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<()> {
        ensure!(
            !key.is_empty()
                && key.chars().count() <= MAX_STATE_KEY_CHARS
                && !key.chars().any(char::is_control),
            "A stored value's key must be 1 to {MAX_STATE_KEY_CHARS} characters, without control characters"
        );
        let text = serde_json::to_string(value)?;
        ensure!(
            text.len() <= MAX_STATE_VALUE_BYTES,
            "A stored value can be at most 16 KiB"
        );
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        definition(&tx, script)?;
        if value.is_null() {
            tx.execute(
                "DELETE FROM script_state WHERE script_id=?1 AND key=?2",
                params![script, key],
            )?;
        } else {
            let (count, bytes): (usize, usize) = tx.query_row(
                "SELECT COUNT(*),COALESCE(SUM(length(CAST(key AS BLOB))+length(CAST(value AS BLOB))),0)
                 FROM script_state WHERE script_id=?1 AND key<>?2",
                params![script, key],
                |r| Ok((r.get::<_, i64>(0)? as usize, r.get::<_, i64>(1)? as usize)),
            )?;
            ensure!(
                count < MAX_STATE_KEYS,
                "A script can keep at most {MAX_STATE_KEYS} values; remove some with xd.store.set(key, nil)"
            );
            ensure!(
                bytes + key.len() + text.len() <= MAX_STATE_BYTES,
                "A script can keep at most 256 KiB of stored values"
            );
            tx.execute(
                "INSERT INTO script_state(script_id,key,value,updated) VALUES(?1,?2,?3,?4)
                 ON CONFLICT(script_id,key) DO UPDATE SET value=excluded.value,updated=excluded.updated",
                params![script, key, text, chrono::Utc::now().timestamp()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    /// Forget everything a script stored. Returns how many values were removed.
    pub fn clear_script_values(&self, script: &str) -> Result<usize> {
        Ok(self
            .connect()?
            .execute("DELETE FROM script_state WHERE script_id=?1", [script])?)
    }
    /// Count one script retry or restart of `job`, refusing the sixth until it completes.
    pub(crate) fn count_script_retry(&self, job: &str) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let count: i64 = tx
            .query_row(
                "SELECT count FROM script_retries WHERE job_id=?1",
                [job],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);
        ensure!(
            count < MAX_SCRIPT_RETRIES,
            "Scripts already retried this download {MAX_SCRIPT_RETRIES} times. Retry it yourself, or remove and add it again"
        );
        tx.execute(
            "INSERT INTO script_retries(job_id,count) VALUES(?1,1)
             ON CONFLICT(job_id) DO UPDATE SET count=count+1",
            [job],
        )?;
        tx.commit()?;
        Ok(())
    }
    /// Give back a counted retry the scheduler then refused.
    pub(crate) fn uncount_script_retry(&self, job: &str) -> Result<()> {
        self.connect()?.execute(
            "UPDATE script_retries SET count=count-1 WHERE job_id=?1 AND count>0",
            [job],
        )?;
        Ok(())
    }
    /// Count one notification for a running script, refusing past the limit.
    pub(crate) fn count_script_notice(&self, run: &str) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (state, count): (String, i64) = tx
            .query_row(
                "SELECT state,notices FROM script_runs WHERE id=?1",
                [run],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .context("Run no longer exists")?;
        ensure!(
            state == "running",
            "Only a running script can show a notification"
        );
        ensure!(
            count < MAX_NOTICES,
            "A script can show at most {MAX_NOTICES} notifications per run"
        );
        tx.execute(
            "UPDATE script_runs SET notices=notices+1 WHERE id=?1",
            [run],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn script_runs(&self, script: Option<&str>) -> Result<Vec<Run>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT data FROM script_runs WHERE (?1 IS NULL OR script_id=?1) ORDER BY rowid DESC",
        )?;
        let data = stmt
            .query_map([script], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        data.into_iter()
            .map(|v| Ok(serde_json::from_str(&v)?))
            .collect()
    }
    pub fn script_run(&self, id: &str) -> Result<Run> {
        read_run(&self.connect()?, id)
    }
    pub fn script_logs(&self, id: &str) -> Result<Vec<Log>> {
        let conn = self.connect()?;
        read_run(&conn, id)?;
        let mut stmt =
            conn.prepare("SELECT at,level,text FROM script_logs WHERE run_id=?1 ORDER BY seq")?;
        Ok(stmt
            .query_map([id], |r| {
                Ok(Log {
                    at: r.get(0)?,
                    level: r.get(1)?,
                    text: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn stop_script(&self, id: &str) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut run = read_run(&tx, id)?;
        if run.state == "queued" {
            run.state = "stopped".into();
            run.finished_at = Some(chrono::Utc::now().timestamp());
            update_run(&tx, &run)?;
        } else if run.state == "running" {
            tx.execute("UPDATE script_runs SET stop_requested=1 WHERE id=?1", [id])?;
        }
        prune_runs(&tx)?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn script_stop_requested(&self, id: &str) -> Result<bool> {
        Ok(self.connect()?.query_row(
            "SELECT stop_requested FROM script_runs WHERE id=?1",
            [id],
            |r| r.get(0),
        )?)
    }
    pub(crate) fn recover_scripts(&self) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ids = {
            let mut s =
                tx.prepare("SELECT id FROM script_runs WHERE state IN ('queued','running')")?;
            s.query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        for id in ids {
            let mut run = read_run(&tx, &id)?;
            run.state = "interrupted".into();
            run.error = Some(
                "App stopped before this run finished. It will not replay automatically.".into(),
            );
            run.finished_at = Some(chrono::Utc::now().timestamp());
            update_run(&tx, &run)?;
        }
        let now = chrono::Utc::now().timestamp();
        for script in definitions(&tx)? {
            tx.execute(
                "UPDATE automation_scripts SET cursor=?1 WHERE id=?2",
                params![latest(&tx)?, script.id],
            )?;
            if let Trigger::Interval { minutes } = script.trigger {
                tx.execute(
                    "UPDATE automation_scripts SET next_at=?1 WHERE id=?2 AND next_at<=?3",
                    params![now + i64::from(minutes) * 60, script.id, now],
                )?;
            }
        }
        prune_runs(&tx)?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn schedule_scripts(&self, now: i64, startup: bool) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let end = latest(&tx)?;
        for script in definitions(&tx)?.into_iter().filter(|s| s.enabled) {
            if busy(&tx, &script.id)? {
                continue;
            }
            let (cursor, next): (i64, Option<i64>) = tx.query_row(
                "SELECT cursor,next_at FROM automation_scripts WHERE id=?1",
                [&script.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let mut context = None;
            match &script.trigger {
                // Site readers run only when a matching page is opened or tested.
                Trigger::Manual | Trigger::SiteReader { .. } => {}
                Trigger::Startup if startup => {
                    context = Some(json!({"trigger":"startup","downloads":[]}))
                }
                Trigger::Startup => {}
                Trigger::Interval { minutes } if next.is_some_and(|at| at <= now) => {
                    context = Some(json!({"trigger":"interval","downloads":[]}));
                    tx.execute(
                        "UPDATE automation_scripts SET next_at=?1 WHERE id=?2",
                        params![now + i64::from(*minutes) * 60, script.id],
                    )?;
                }
                Trigger::Interval { .. } => {}
                trigger @ (Trigger::DownloadCompleted { .. }
                | Trigger::DownloadFailed { .. }
                | Trigger::QueueCompleted { .. }) => {
                    if end <= cursor {
                        continue;
                    }
                    let (name, kind, queue) = match trigger {
                        Trigger::DownloadCompleted { queue } => {
                            ("download_completed", "completed", queue.as_deref())
                        }
                        Trigger::DownloadFailed { queue } => {
                            ("download_failed", "failed", queue.as_deref())
                        }
                        Trigger::QueueCompleted { queue } => {
                            ("queue_completed", "completed", Some(queue.as_str()))
                        }
                        _ => unreachable!(),
                    };
                    let mut stmt=tx.prepare("SELECT e.seq,e.job_id,e.queue_id,j.data FROM script_events e LEFT JOIN jobs j ON j.id=e.job_id WHERE e.seq>?1 AND e.kind=?3 AND (?2 IS NULL OR e.queue_id=?2) ORDER BY e.seq LIMIT 100")?;
                    let rows = stmt
                        .query_map(params![cursor, queue, kind], |r| {
                            Ok((
                                r.get::<_, i64>(0)?,
                                r.get::<_, String>(1)?,
                                r.get::<_, String>(2)?,
                                r.get::<_, Option<String>>(3)?,
                            ))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    let complete = if let Trigger::QueueCompleted { queue } = trigger {
                        tx.query_row("SELECT COUNT(*)>0 AND COALESCE(SUM(json_extract(j.data,'$.status')<>'complete'),0)=0 FROM jobs j JOIN job_queues q ON q.job_id=j.id WHERE q.queue_id=?1",[queue],|r|r.get::<_,bool>(0))?
                    } else {
                        true
                    };
                    if !rows.is_empty() && complete {
                        let mut downloads = Vec::new();
                        for (_, id, queue, data) in &rows {
                            let job = data
                                .as_ref()
                                .map(|s| serde_json::from_str::<crate::model::Job>(s))
                                .transpose()?;
                            downloads.push(event_download(kind, id, queue, job.as_ref()));
                        }
                        let first: i64 = tx.query_row(
                            "SELECT COALESCE(MIN(seq),0) FROM script_events",
                            [],
                            |r| r.get(0),
                        )?;
                        // A queue event identifies the queue. It is not a partial
                        // list of its files (the event batch itself is bounded).
                        if matches!(trigger, Trigger::QueueCompleted { .. }) {
                            downloads.clear();
                        }
                        context = Some(
                            json!({"trigger":name,"queue":queue,"downloads":downloads,"events_expired":cursor<first-1}),
                        );
                    }
                    // Queue completion waits for its remaining jobs, even across ticks.
                    if complete || rows.is_empty() {
                        let consumed = if matches!(trigger, Trigger::QueueCompleted { .. }) {
                            end
                        } else {
                            rows.last().map_or(end, |r| r.0)
                        };
                        tx.execute(
                            "UPDATE automation_scripts SET cursor=?1 WHERE id=?2",
                            params![consumed, script.id],
                        )?;
                    }
                }
            }
            if let Some(context) = context {
                enqueue(
                    &tx,
                    &script,
                    &uuid::Uuid::new_v4().to_string(),
                    context,
                    now,
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }
    /// Someone waits on a site reader, so readers go first; `readers_only`
    /// claims from the slot kept free for them.
    pub(crate) fn claim_script(&self, readers_only: bool) -> Result<Option<Claim>> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row: Option<(String, String, u32)> = tx
            .query_row(
                "SELECT id,body,timeout FROM script_runs WHERE state='queued'
                 AND (?1=0 OR json_extract(data,'$.trigger')='site_reader')
                 ORDER BY json_extract(data,'$.trigger')='site_reader' DESC,rowid LIMIT 1",
                [readers_only],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((id, body, timeout)) = row else {
            return Ok(None);
        };
        let mut run = read_run(&tx, &id)?;
        run.state = "running".into();
        run.started_at = Some(chrono::Utc::now().timestamp());
        update_run(&tx, &run)?;
        tx.commit()?;
        Ok(Some(Claim { run, body, timeout }))
    }
    pub(crate) fn finish_script(
        &self,
        id: &str,
        state: &str,
        error: Option<String>,
        exit_code: Option<i32>,
    ) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut run = read_run(&tx, id)?;
        if run.active() {
            run.state = state.into();
            run.error = error;
            run.exit_code = exit_code;
            run.finished_at = Some(chrono::Utc::now().timestamp());
            update_run(&tx, &run)?;
        }
        prune_runs(&tx)?;
        tx.commit()?;
        Ok(())
    }
    /// The first enabled site reader whose pages cover `page`, in the order the
    /// scripts were created. Built-in readers always take their own sites.
    pub fn site_reader_for(&self, page: &str) -> Result<Option<Script>> {
        if crate::hls::is_site_video(page) {
            return Ok(None);
        }
        let Ok(url) = crate::model::validate_url(page) else {
            return Ok(None);
        };
        Ok(definitions(&self.connect()?)?.into_iter().find(|script| {
            script.enabled
                && script.language == Language::Lua
                && matches!(&script.trigger, Trigger::SiteReader { pages }
                    if pages.iter().any(|p| site::page_matches(p, &url)))
        }))
    }
    /// Read `page` with a saved site reader. Retrying the same request ID
    /// returns the same run.
    pub fn run_site_reader(&self, id: &str, page: &str, request: &str) -> Result<Run> {
        crate::model::validate_job_id(request)?;
        let page = crate::model::validate_url(page)?.to_string();
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM script_runs WHERE id=?1)",
            [request],
            |r| r.get::<_, bool>(0),
        )? {
            let run = read_run(&tx, request)?;
            ensure!(run.script_id == id, "Run ID belongs to another script");
            return Ok(run);
        }
        let script = definition(&tx, id)?;
        ensure!(
            matches!(script.trigger, Trigger::SiteReader { .. })
                && script.language == Language::Lua,
            "This script is not a site reader"
        );
        let run = enqueue(
            &tx,
            &script,
            request,
            json!({"trigger":"site_reader","url":page,"downloads":[]}),
            chrono::Utc::now().timestamp(),
        )?;
        tx.commit()?;
        Ok(run)
    }
    /// What a finished site reader run found, if it succeeded.
    pub fn script_result(&self, run: &str) -> Result<Option<Reading>> {
        let data: Option<String> = self
            .connect()?
            .query_row(
                "SELECT data FROM script_results WHERE run_id=?1",
                [run],
                |r| r.get(0),
            )
            .optional()?;
        data.map(|data| {
            let reading: Reading = serde_json::from_str(&data)?;
            reading.check()?;
            Ok(reading)
        })
        .transpose()
    }
    pub(crate) fn save_script_result(&self, run: &str, reading: &Reading) -> Result<()> {
        reading.check()?;
        self.connect()?.execute(
            "INSERT INTO script_results(run_id,data) VALUES(?1,?2)
             ON CONFLICT(run_id) DO UPDATE SET data=excluded.data",
            params![run, serde_json::to_string(reading)?],
        )?;
        Ok(())
    }
    pub(crate) fn append_script_log(&self, id: &str, level: &str, text: &str) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (bytes, truncated): (usize, bool) = tx.query_row(
            "SELECT log_bytes,truncated FROM script_runs WHERE id=?1",
            [id],
            |r| Ok((r.get::<_, u32>(0)? as usize, r.get(1)?)),
        )?;
        if truncated {
            return Ok(());
        }
        let mut end = text.len().min(MAX_LOG_BYTES.saturating_sub(bytes));
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let now = chrono::Utc::now().timestamp();
        if end > 0 {
            tx.execute(
                "INSERT INTO script_logs(run_id,at,level,text) VALUES(?1,?2,?3,?4)",
                params![id, now, level, &text[..end]],
            )?;
        }
        if end < text.len() {
            tx.execute("INSERT INTO script_logs(run_id,at,level,text) VALUES(?1,?2,'warn','Output truncated at 128 KiB. The script continues running.')",params![id,now])?;
        }
        tx.execute(
            "UPDATE script_runs SET log_bytes=?1,truncated=?2 WHERE id=?3",
            params![(bytes + end) as u32, end < text.len(), id],
        )?;
        tx.commit()?;
        Ok(())
    }
}

/// Read `page` with a saved site reader and wait for what it found. The
/// scheduler runs it like any script (logs, Stop, timeout); cancelling stops it.
pub async fn read_page(
    store: &Store,
    script: &str,
    page: &str,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<Reading> {
    let run = store.run_site_reader(script, page, &uuid::Uuid::new_v4().to_string())?;
    let started = std::time::Instant::now();
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                store.stop_script(&run.id)?;
                anyhow::bail!("Site reader cancelled");
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(150)) => {}
        }
        let current = store.script_run(&run.id)?;
        if current.state == "queued" {
            ensure!(
                started.elapsed() < std::time::Duration::from_secs(60),
                "The site reader did not start. Make sure Download Engine is open."
            );
            continue;
        }
        if current.active() {
            continue;
        }
        if current.state == "succeeded" {
            return store
                .script_result(&run.id)?
                .context("The site reader finished without a result");
        }
        // The script's own error is in its log; the run only knows the exit code.
        let errors: Vec<String> = store
            .script_logs(&run.id)?
            .into_iter()
            .filter(|log| log.level == "error")
            .map(|log| log.text)
            .collect();
        // Lua errors end in a stack trace; the first line is the message.
        let detail = errors.concat();
        let message = match detail.lines().map(str::trim).find(|l| !l.is_empty()) {
            Some(line) => line.chars().take(300).collect(),
            None => current
                .error
                .unwrap_or_else(|| format!("The site reader ended: {}", current.state)),
        };
        anyhow::bail!("{}: {message}", current.name);
    }
}
