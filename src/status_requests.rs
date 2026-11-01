//! One UI request per browser handoff, including native-host process restarts.
//! Receipt retention prevents a repeated native reply from reopening a hidden window.
use crate::store::Store;
use anyhow::Result;
use rusqlite::Connection;

pub(crate) fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS download_status_requests (
        job TEXT PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
        consumed INTEGER NOT NULL DEFAULT 0 CHECK(consumed IN (0,1)));",
    )?;
    #[cfg(unix)]
    conn.execute_batch("CREATE TABLE IF NOT EXISTS window_activation (
        singleton INTEGER PRIMARY KEY CHECK(singleton=1), browser INTEGER NOT NULL CHECK(browser IN (0,1)));")?;
    Ok(())
}

#[cfg(unix)]
impl Store {
    pub fn request_window_activation(&self, browser: bool) -> Result<()> {
        self.connect()?.execute(
            "INSERT INTO window_activation(singleton,browser) VALUES(1,?1)
            ON CONFLICT(singleton) DO UPDATE SET browser=MAX(browser,excluded.browser)",
            [browser],
        )?;
        Ok(())
    }
    pub fn take_window_activation(&self) -> Result<Option<bool>> {
        use rusqlite::OptionalExtension;
        let mut conn = self.connect()?;
        let pending: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM window_activation)",
            [],
            |row| row.get(0),
        )?;
        if !pending {
            return Ok(None);
        }
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let browser = tx
            .query_row(
                "SELECT browser FROM window_activation WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        tx.execute("DELETE FROM window_activation WHERE singleton=1", [])?;
        tx.commit()?;
        Ok(browser)
    }
}
impl Store {
    pub fn request_download_status(&self, id: &str) -> Result<()> {
        crate::model::validate_job_id(id)?;
        self.connect()?.execute(
            "INSERT OR IGNORE INTO download_status_requests(job) SELECT id FROM jobs WHERE id=?1",
            [id],
        )?;
        Ok(())
    }
    pub fn take_download_status_requests(&self) -> Result<Vec<String>> {
        let mut conn = self.connect()?;
        let pending: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM download_status_requests WHERE consumed=0)",
            [],
            |row| row.get(0),
        )?;
        if !pending {
            return Ok(Vec::new());
        }
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let jobs = tx
            .prepare("SELECT job FROM download_status_requests WHERE consumed=0")?
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        tx.execute(
            "UPDATE download_status_requests SET consumed=1 WHERE consumed=0",
            [],
        )?;
        tx.commit()?;
        Ok(jobs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model::Job, paths::AppPaths};
    #[cfg(unix)]
    #[test]
    fn window_activation_coalesces_and_is_consumed_once() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(AppPaths {
            data: root.path().join("profile"),
            downloads: root.path().join("files"),
        })
        .unwrap();
        assert_eq!(store.take_window_activation().unwrap(), None);
        store.request_window_activation(false).unwrap();
        store.request_window_activation(true).unwrap();
        store.request_window_activation(false).unwrap();
        assert_eq!(store.take_window_activation().unwrap(), Some(true));
        assert_eq!(store.take_window_activation().unwrap(), None);
        assert!(store.list().unwrap().is_empty());
    }
    #[test]
    fn browser_status_request_is_consumed_once_across_process_connections() {
        let root = tempfile::tempdir().unwrap();
        let paths = AppPaths {
            data: root.path().join("data"),
            downloads: root.path().join("downloads"),
        };
        let host = Store::open(paths.clone()).unwrap();
        let app = Store::open(paths).unwrap();
        let job = Job::new(
            "https://example.com/file.zip",
            None,
            host.paths.downloads.clone(),
            "fixture".into(),
        )
        .unwrap();
        host.enqueue(&job).unwrap();
        host.request_download_status(&job.id).unwrap();
        assert_eq!(
            app.take_download_status_requests().unwrap(),
            vec![job.id.clone()]
        );
        host.request_download_status(&job.id).unwrap();
        assert!(app.take_download_status_requests().unwrap().is_empty());
        assert_eq!(host.list().unwrap().len(), 1);
    }
}
