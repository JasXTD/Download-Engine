//! Short-lived recovery intents. Browser cancellation follows a durable receipt,
//! committed in the same transaction as the existing download's replacement URL.
use crate::{
    model::{Job, Status, validate_url},
    store::Store,
};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub url: String,
    pub filename: String,
    pub size: u64,
    pub mime: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub job_id: String,
    pub name: String,
    pub page: String,
    pub size: Option<u64>,
    pub state: String,
    pub candidate: Option<Candidate>,
    pub expires: i64,
}
pub(crate) fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS browser_recovery(
        id TEXT PRIMARY KEY,job TEXT NOT NULL,name TEXT NOT NULL,page TEXT NOT NULL,size INTEGER,
        state TEXT NOT NULL,candidate TEXT,expires INTEGER NOT NULL,lease INTEGER NOT NULL);",
    )?;
    Ok(())
}
fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
fn cleanup(conn: &Connection) -> Result<()> {
    conn.execute("UPDATE browser_recovery SET state='cancelled',candidate=NULL WHERE state IN ('waiting','offered') AND (expires<=?1 OR lease<=?1 OR NOT EXISTS(SELECT 1 FROM jobs WHERE jobs.id=browser_recovery.job))",[now()])?;
    conn.execute(
        "DELETE FROM browser_recovery WHERE state IN ('accepted','cancelled') AND expires<?1",
        [now() - 86400],
    )?;
    Ok(())
}
fn read(conn: &Connection, id: &str) -> Result<Option<Session>> {
    let value = conn
        .query_row(
            "SELECT job,name,page,size,state,candidate,expires FROM browser_recovery WHERE id=?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            },
        )
        .optional()?;
    value
        .map(|(job_id, name, page, size, state, raw, expires)| {
            Ok(Session {
                id: Uuid::parse_str(id)?,
                job_id,
                name,
                page,
                size: size.map(u64::try_from).transpose()?,
                state,
                candidate: raw.map(|v| serde_json::from_str(&v)).transpose()?,
                expires,
            })
        })
        .transpose()
}
fn inactive(job: &Job) -> Result<()> {
    ensure!(
        job.hls.is_none() && job.ftp.is_none(),
        "Browser link recovery is for direct HTTP/HTTPS files"
    );
    ensure!(
        job.saved_path.is_none(),
        "This download already has a saved file"
    );
    ensure!(
        matches!(job.status, Status::Paused | Status::Failed),
        "Pause the download before finding a replacement link"
    );
    Ok(())
}
impl Store {
    pub fn begin_browser_recovery(&self, job_id: &str, page: &str) -> Result<Lease> {
        let page = validate_url(page)?.to_string();
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let job: Job = serde_json::from_str(&tx.query_row::<String, _, _>(
            "SELECT data FROM jobs WHERE id=?1",
            [job_id],
            |r| r.get(0),
        )?)?;
        inactive(&job)?;
        cleanup(&tx)?;
        tx.execute("UPDATE browser_recovery SET state='cancelled',candidate=NULL WHERE state IN ('waiting','offered')",[])?;
        let id = Uuid::new_v4();
        tx.execute("INSERT INTO browser_recovery(id,job,name,page,size,state,expires,lease) VALUES(?1,?2,?3,?4,?5,'waiting',?6,?7)",params![id.to_string(),job.id,job.name,page,job.total.map(i64::try_from).transpose()?,now()+1800,now()+60])?;
        tx.commit()?;
        Ok(Lease {
            id,
            store: self.clone(),
        })
    }
    pub fn pending_browser_recovery(&self) -> Result<Option<Session>> {
        let conn = self.connect()?;
        cleanup(&conn)?;
        let id:Option<String>=conn.query_row("SELECT id FROM browser_recovery WHERE state IN ('waiting','offered') ORDER BY rowid DESC LIMIT 1",[],|r|r.get(0)).optional()?;
        id.map(|id| read(&conn, &id))
            .transpose()
            .map(Option::flatten)
    }
    pub fn browser_recovery_status(&self, id: Uuid) -> Result<Option<Session>> {
        let conn = self.connect()?;
        cleanup(&conn)?;
        read(&conn, &id.to_string())
    }
    pub fn offer_browser_recovery(&self, id: Uuid, mut candidate: Candidate) -> Result<Session> {
        candidate.url = validate_url(&candidate.url)?.to_string();
        crate::capture::ExpectedFile::browser(candidate.size, &candidate.mime)?;
        ensure!(
            !candidate.filename.is_empty()
                && candidate.filename.len() <= 1024
                && !candidate.filename.contains(['\\', '/'])
                && !candidate.filename.chars().any(char::is_control),
            "Invalid browser filename"
        );
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        cleanup(&tx)?;
        let session = read(&tx, &id.to_string())?.context("Recovery request no longer exists")?;
        if session.state == "offered" && session.candidate.as_ref() == Some(&candidate) {
            return Ok(session);
        }
        ensure!(
            session.state == "waiting",
            "Recovery is already handled or no longer waiting"
        );
        let job: Job = serde_json::from_str(&tx.query_row::<String, _, _>(
            "SELECT data FROM jobs WHERE id=?1",
            [&session.job_id],
            |r| r.get(0),
        )?)?;
        inactive(&job)?;
        ensure!(
            session.size.is_none_or(|size| size == candidate.size),
            "This browser download has a different size; it was kept in the browser"
        );
        tx.execute(
            "UPDATE browser_recovery SET state='offered',candidate=?1 WHERE id=?2",
            params![serde_json::to_string(&candidate)?, id.to_string()],
        )?;
        let saved = read(&tx, &id.to_string())?.unwrap();
        tx.commit()?;
        Ok(saved)
    }
    pub fn cancel_browser_recovery(&self, id: Uuid) -> Result<Option<Session>> {
        let conn = self.connect()?;
        conn.execute("UPDATE browser_recovery SET state='cancelled',candidate=NULL WHERE id=?1 AND state IN ('waiting','offered')",[id.to_string()])?;
        read(&conn, &id.to_string())
    }
}
pub struct Lease {
    pub id: Uuid,
    store: Store,
}
impl Lease {
    pub fn refresh(&self) -> Result<Session> {
        let conn = self.store.connect()?;
        cleanup(&conn)?;
        conn.execute(
            "UPDATE browser_recovery SET lease=?1 WHERE id=?2 AND state IN ('waiting','offered')",
            params![now() + 60, self.id.to_string()],
        )?;
        read(&conn, &self.id.to_string())?.context("Browser recovery no longer exists")
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let _ = self.store.cancel_browser_recovery(self.id);
    }
}
pub(crate) fn accept(conn: &Connection, id: Uuid, job: &str, url: &str) -> Result<String> {
    cleanup(conn)?;
    let session = read(conn, &id.to_string())?.context("Recovery request no longer exists")?;
    ensure!(
        session.state == "offered"
            && session.job_id == job
            && session.candidate.as_ref().is_some_and(|c| c.url == url),
        "Browser recovery ended or changed. Find a fresh link again"
    );
    conn.execute(
        "UPDATE browser_recovery SET state='accepted',candidate=NULL WHERE id=?1",
        [id.to_string()],
    )?;
    Ok(session.page)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_is_bounded_idempotent_and_never_turns_cancelled_into_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(crate::paths::AppPaths {
            data: dir.path().join("data"),
            downloads: dir.path().join("downloads"),
        })
        .unwrap();
        let mut job = Job::new(
            "https://files.test/old.zip",
            Some("file.zip"),
            store.paths.downloads.clone(),
            "old".into(),
        )
        .unwrap();
        job.status = Status::Paused;
        job.total = Some(10);
        store.enqueue(&job).unwrap();
        let lease = store
            .begin_browser_recovery(&job.id, "https://site.test/page")
            .unwrap();
        assert_eq!(
            store.pending_browser_recovery().unwrap().unwrap().id,
            lease.id
        );
        let candidate = Candidate {
            url: "https://files.test/new.zip".into(),
            filename: "file.zip".into(),
            size: 10,
            mime: "application/zip".into(),
        };
        assert_eq!(
            store
                .offer_browser_recovery(lease.id, candidate.clone())
                .unwrap()
                .state,
            "offered"
        );
        assert_eq!(
            store
                .offer_browser_recovery(lease.id, candidate.clone())
                .unwrap()
                .state,
            "offered"
        );
        let mut other = candidate.clone();
        other.url = "https://files.test/other.zip".into();
        assert!(store.offer_browser_recovery(lease.id, other).is_err());
        store.cancel_browser_recovery(lease.id).unwrap();
        assert!(accept(&store.connect().unwrap(), lease.id, &job.id, &candidate.url).is_err());
        assert_eq!(store.get(&job.id).unwrap().unwrap().url, job.url);
        let lease = store
            .begin_browser_recovery(&job.id, "https://site.test/page")
            .unwrap();
        store
            .connect()
            .unwrap()
            .execute(
                "UPDATE browser_recovery SET lease=0 WHERE id=?1",
                [lease.id.to_string()],
            )
            .unwrap();
        assert!(store.pending_browser_recovery().unwrap().is_none());
        assert_eq!(lease.refresh().unwrap().state, "cancelled");
        assert!(store.offer_browser_recovery(lease.id, candidate).is_err());
    }
}
