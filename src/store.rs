use crate::{
    model::{Job, Status},
    paths::AppPaths,
};
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone)]
pub struct Store {
    pub paths: AppPaths,
    pub activity: crate::transfer_activity::Activity,
    db: PathBuf,
    // Keep WAL alive and reuse a connection for frequent progress/quota writes.
    // No transaction is held between operations; other readers remain independent.
    lifetime_connection: Arc<Mutex<Connection>>,
}
impl Store {
    pub fn open(paths: AppPaths) -> Result<Self> {
        paths.prepare()?;
        let db = paths.data.join("queue.sqlite3");
        let conn = connect_database(&db)?;
        let store = Self {
            db,
            paths,
            activity: Default::default(),
            lifetime_connection: Arc::new(Mutex::new(conn)),
        };
        let conn = store
            .lifetime_connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database initialization was interrupted"))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch("CREATE TABLE IF NOT EXISTS jobs (id TEXT PRIMARY KEY, request_id TEXT NOT NULL UNIQUE, created INTEGER NOT NULL, data TEXT NOT NULL);")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS browser_captures (
            request_id TEXT PRIMARY KEY, state TEXT NOT NULL, data TEXT, job_id TEXT);",
        )?;
        store.initialize_queues()?;
        crate::queue_start::initialize(&conn)?;
        crate::auth::initialize(&conn)?;
        crate::bandwidth::initialize(&conn)?;
        crate::quota::initialize(&conn)?;
        crate::preferences::initialize(&conn)?;
        crate::proxy::initialize(&conn)?;
        crate::categories::initialize(&conn)?;
        crate::browser_batch::initialize(&conn)?;
        crate::browser_video::initialize(&conn)?;
        crate::grabber_projects::initialize(&conn)?;
        crate::grabber_schedule::initialize(&conn)?;
        crate::hls::initialize(&conn)?;
        crate::antivirus::initialize(&conn)?;
        crate::automation::initialize(&conn)?;
        crate::script_automation::initialize(&conn)?;
        crate::control::initialize(&conn)?;
        crate::mcp::initialize(&conn)?;
        crate::connections::initialize(&conn)?;
        crate::browser_recovery::initialize(&conn)?;
        crate::status_requests::initialize(&conn)?;
        drop(conn);
        Ok(store)
    }
    pub(crate) fn connect(&self) -> Result<Connection> {
        connect_database(&self.db)
    }
    pub(crate) fn with_connection<T>(
        &self,
        operation: impl FnOnce(&mut Connection) -> Result<T>,
    ) -> Result<T> {
        let mut conn = self
            .lifetime_connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database writer was interrupted"))?;
        operation(&mut conn)
    }
    /// Progress checkpoints must not block a Tokio network worker on SQLite I/O.
    pub(crate) async fn checkpoint(&self, job: &Job) -> Result<()> {
        let store = self.clone();
        let job = job.clone();
        tokio::task::spawn_blocking(move || store.save(&job)).await?
    }
    /// Acknowledge a handoff only after this durable insert succeeds.
    pub fn enqueue(&self, job: &Job) -> Result<Job> {
        Ok(self
            .enqueue_batch(std::slice::from_ref(job), crate::queue::MAIN_QUEUE)?
            .remove(0))
    }

    /// Reservations never appear in the runnable queue. Terminal receipts survive
    /// removal of a job so a browser recovering a lost reply can still resolve it.
    pub fn capture_status(&self, request_id: &str) -> Result<CaptureReceipt> {
        capture_receipt(&self.connect()?, request_id)
    }
    pub fn prepare_capture(&self, job: &Job) -> Result<CaptureReceipt> {
        job.validate_persisted()?;
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let existing = capture_receipt(&tx, &job.request_id)?;
        match existing.state {
            CaptureState::Missing => {
                tx.execute("INSERT INTO browser_captures (request_id,state,data,job_id) VALUES (?1,'prepared',?2,?3)",
                    params![job.request_id, serde_json::to_string(job)?, job.id])?;
            }
            CaptureState::Aborted => anyhow::bail!("Browser handoff was abandoned"),
            CaptureState::Committed => return Ok(existing),
            _ => {
                let saved = existing.job.as_ref().context("Missing capture data")?;
                anyhow::ensure!(
                    saved.url == job.url && saved.expected_file == job.expected_file,
                    "Capture identifier belongs to a different file"
                );
                return Ok(existing);
            }
        }
        tx.commit()?;
        Ok(CaptureReceipt {
            state: CaptureState::Prepared,
            job: Some(job.clone()),
            job_id: Some(job.id.clone()),
        })
    }
    pub fn commit_capture(&self, request_id: &str) -> Result<CaptureReceipt> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut receipt = capture_receipt(&tx, request_id)?;
        match receipt.state {
            CaptureState::Committed => return Ok(receipt),
            CaptureState::Prepared => {}
            _ => anyhow::bail!("No prepared browser handoff exists"),
        }
        let job = receipt.job.as_mut().context("Missing capture data")?;
        job.status = Status::Queued;
        tx.execute(
            "INSERT INTO jobs (id,request_id,created,data) VALUES (?1,?2,?3,?4)",
            params![
                job.id,
                job.request_id,
                i64::try_from(job.created)?,
                serde_json::to_string(job)?
            ],
        )?;
        crate::queue_store::assign(&tx, &job.id, crate::queue::MAIN_QUEUE)?;
        crate::automation::on_added(&tx, job)?;
        crate::queue_start::wake_idle_main(&tx, &job.id)?;
        tx.execute(
            "UPDATE browser_captures SET state='committed',data=NULL WHERE request_id=?1",
            [request_id],
        )?;
        tx.commit()?;
        receipt.state = CaptureState::Committed;
        Ok(receipt)
    }
    pub fn abort_capture(&self, request_id: &str) -> Result<CaptureReceipt> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // A tombstone also prevents an in-flight prepare from reviving an aborted handoff.
        tx.execute(
            "INSERT INTO browser_captures (request_id,state,data) VALUES (?1,'aborted',NULL)
            ON CONFLICT(request_id) DO UPDATE SET state='aborted',data=NULL,job_id=NULL
            WHERE browser_captures.state='prepared'",
            [request_id],
        )?;
        let receipt = capture_receipt(&tx, request_id)?;
        tx.commit()?;
        Ok(receipt)
    }

    pub fn list(&self) -> Result<Vec<Job>> {
        let conn = self.connect()?;
        let mut stmt =
            conn.prepare("SELECT id,data FROM jobs ORDER BY created DESC, rowid DESC")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        rows.map(|row| {
            let (id, data) = row?;
            decode_job(&id, &data)
        })
        .collect()
    }
    pub fn get(&self, id: &str) -> Result<Option<Job>> {
        if crate::model::validate_job_id(id).is_err() {
            // External/UI lookups for stale identifiers behave like any other
            // missing job. Persisted rows are still validated when enumerated.
            return Ok(None);
        }
        let conn = self.connect()?;
        let value: Option<(String, String)> = conn
            .query_row("SELECT id,data FROM jobs WHERE id=?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        value
            .map(|(row_id, data)| decode_job(&row_id, &data))
            .transpose()
    }
    pub fn save(&self, job: &Job) -> Result<()> {
        job.validate_persisted()?;
        let data = serde_json::to_string(job)?;
        let count = self.with_connection(|conn| {
            Ok(conn
                .prepare_cached("UPDATE jobs SET data=?1 WHERE id=?2")?
                .execute(params![data, job.id])?)
        })?;
        anyhow::ensure!(count == 1, "Download no longer exists");
        Ok(())
    }
    pub fn remove(&self, id: &str) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let data: Option<String> = tx
            .query_row("SELECT data FROM jobs WHERE id=?1", [id], |r| r.get(0))
            .optional()?;
        if let Some(data) = data {
            let job: Job = serde_json::from_str(&data)?;
            job.validate_persisted()?;
            anyhow::ensure!(
                job.id == id,
                "Saved download identifier does not match its row"
            );
            if let Some(binding) = &job.authentication {
                crate::auth::retire(&tx, binding)?;
            }
            tx.execute("DELETE FROM jobs WHERE id=?1", [id])?;
        }
        tx.commit()?;
        crate::auth::cleanup_pending(self)
            .context("Download removed, but credential cleanup is pending")
    }
    pub fn recover(&self) -> Result<()> {
        // Start immediately is a one-run override, not a permanent schedule bypass.
        self.connect()?
            .execute("DELETE FROM immediate_downloads", [])?;
        for id in self.queue_paused_jobs()? {
            self.finish_queue_pause(&id)?;
        }
        for mut job in self.list()? {
            if job.status == Status::Downloading {
                job.status = Status::Paused;
                job.retry.next_at = None;
                job.error = Some("Interrupted when the app closed. Resume when ready.".into());
                self.save(&job)?;
            }
        }
        Ok(())
    }
}

pub(crate) fn decode_job(row_id: &str, data: &str) -> Result<Job> {
    crate::model::validate_job_id(row_id)?;
    let job: Job = serde_json::from_str(data).context("Invalid saved job")?;
    job.validate_persisted()?;
    anyhow::ensure!(
        job.id == row_id,
        "Saved download identifier does not match its row"
    );
    Ok(job)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureState {
    Missing,
    Prepared,
    Committed,
    Aborted,
}
pub struct CaptureReceipt {
    pub state: CaptureState,
    pub job: Option<Job>,
    pub job_id: Option<String>,
}
fn capture_receipt(conn: &Connection, request_id: &str) -> Result<CaptureReceipt> {
    let row: Option<(String, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT state,data,job_id FROM browser_captures WHERE request_id=?1",
            [request_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    match row {
        None => Ok(CaptureReceipt {
            state: CaptureState::Missing,
            job: None,
            job_id: None,
        }),
        Some((state, data, job_id)) => {
            if let Some(id) = &job_id {
                crate::model::validate_job_id(id)?;
            }
            let job = data
                .map(|data| -> Result<Job> {
                    let job: Job = serde_json::from_str(&data).context("Invalid saved capture")?;
                    job.validate_persisted()?;
                    if let Some(id) = &job_id {
                        anyhow::ensure!(
                            id == &job.id,
                            "Saved capture identifier does not match its job"
                        );
                    }
                    Ok(job)
                })
                .transpose()?;
            Ok(CaptureReceipt {
                state: match state.as_str() {
                    "prepared" => CaptureState::Prepared,
                    "committed" => CaptureState::Committed,
                    "aborted" => CaptureState::Aborted,
                    _ => anyhow::bail!("Invalid saved capture state"),
                },
                job_id,
                job,
            })
        }
    }
}

fn connect_database(db: &Path) -> Result<Connection> {
    let conn = Connection::open(db)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(conn)
}

/// Add a column to a table made by an older version. `table`, `column` and
/// `definition` are constants. The desktop and CLI processes may open the
/// store at the same moment, so a column another process just added is fine.
pub(crate) fn add_column(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<()> {
    let exists = |conn: &Connection| -> Result<bool> {
        Ok(conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name=?2)",
            params![table, column],
            |r| r.get(0),
        )?)
    };
    if exists(conn)? {
        return Ok(());
    }
    match conn.execute_batch(&format!(
        "ALTER TABLE {table} ADD COLUMN {column} {definition}"
    )) {
        Err(_) if exists(conn)? => Ok(()),
        result => Ok(result?),
    }
}

#[cfg(test)]
mod security_tests {
    use super::*;

    #[test]
    fn corrupt_job_identifiers_are_rejected_before_path_use() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(AppPaths {
            data: temp.path().join("data"),
            downloads: temp.path().join("downloads"),
        })
        .unwrap();
        let mut job = Job::new(
            "https://example.com/file.bin",
            None,
            store.paths.downloads.clone(),
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
        job.id = "../../outside".into();
        store
            .connect()
            .unwrap()
            .execute(
                "INSERT INTO jobs(id,request_id,created,data) VALUES(?1,?2,?3,?4)",
                params![
                    job.id,
                    job.request_id,
                    i64::try_from(job.created).unwrap(),
                    serde_json::to_string(&job).unwrap()
                ],
            )
            .unwrap();

        assert!(store.list().unwrap_err().to_string().contains("identifier"));
        assert!(store.paths.parts(&job.id).is_err());
        assert!(store.get(&job.id).unwrap().is_none());
    }

    #[test]
    fn saved_job_identifier_must_match_its_database_row() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(AppPaths {
            data: temp.path().join("data"),
            downloads: temp.path().join("downloads"),
        })
        .unwrap();
        let job = Job::new(
            "https://example.com/file.bin",
            None,
            store.paths.downloads.clone(),
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
        let row_id = uuid::Uuid::new_v4().to_string();
        store
            .connect()
            .unwrap()
            .execute(
                "INSERT INTO jobs(id,request_id,created,data) VALUES(?1,?2,?3,?4)",
                params![
                    row_id,
                    job.request_id,
                    i64::try_from(job.created).unwrap(),
                    serde_json::to_string(&job).unwrap()
                ],
            )
            .unwrap();

        assert!(
            store
                .get(&row_id)
                .unwrap_err()
                .to_string()
                .contains("does not match")
        );
    }
}
