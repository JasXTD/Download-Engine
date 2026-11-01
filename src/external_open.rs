//! Bounded local inbox for Windows file/protocol activation. Receiving an item
//! only requests a review; it never creates a download or enables torrents.
use crate::store::Store;
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

pub const MAX_PENDING: usize = 8;
const MAX_TEXT: usize = 32_768;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Target {
    TorrentFile(PathBuf),
    Magnet(String),
}
impl Target {
    pub fn parse(value: &OsStr, base: &Path) -> Result<Self> {
        let text = value
            .to_str()
            .context("The torrent path is not valid Unicode")?;
        ensure!(
            !text.is_empty() && text.len() <= MAX_TEXT && !text.chars().any(char::is_control),
            "Invalid torrent path or magnet link"
        );
        if crate::torrent::is_magnet(text) {
            crate::torrent::info_hash(text)?;
            return Ok(Self::Magnet(text.to_owned()));
        }
        ensure!(
            !text.contains("://") && !text.starts_with("--"),
            "Open a local .torrent file or a magnet link"
        );
        let path = Path::new(text);
        ensure!(
            path.extension()
                .is_some_and(|v| v.eq_ignore_ascii_case("torrent")),
            "Open a .torrent file or a magnet link"
        );
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            base.join(path)
        };
        ensure!(path.is_absolute(), "The torrent file path must be absolute");
        // Do not read the torrent or resolve network folders during activation.
        // The existing background metadata reader reports missing/invalid files.
        let target = Self::TorrentFile(path);
        target.validate()?;
        Ok(target)
    }
    fn validate(&self) -> Result<()> {
        let text = self.text()?;
        ensure!(
            text.len() <= MAX_TEXT && !text.chars().any(char::is_control),
            "Invalid external torrent request"
        );
        match self {
            Self::Magnet(link) => {
                crate::torrent::info_hash(link)?;
            }
            Self::TorrentFile(path) => ensure!(
                path.is_absolute()
                    && path
                        .extension()
                        .is_some_and(|v| v.eq_ignore_ascii_case("torrent")),
                "Open a local .torrent file"
            ),
        }
        Ok(())
    }
    pub fn text(&self) -> Result<&str> {
        match self {
            Self::Magnet(link) => Ok(link),
            Self::TorrentFile(path) => path
                .to_str()
                .context("The torrent path is not valid Unicode"),
        }
    }
    fn key(&self) -> Result<String> {
        match self {
            Self::Magnet(link) => Ok(format!("magnet:{}", crate::torrent::info_hash(link)?)),
            Self::TorrentFile(path) => {
                let value = path
                    .to_str()
                    .context("The torrent path is not valid Unicode")?;
                Ok(format!(
                    "file:{}",
                    if cfg!(windows) {
                        value.to_lowercase()
                    } else {
                        value.to_owned()
                    }
                ))
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Pending {
    pub id: String,
    pub target: Target,
}
pub struct Inbox {
    pub next: Option<Pending>,
    pub count: usize,
    pub newly_received: usize,
}
#[derive(Clone, Debug)]
pub struct Submission {
    pub id: String,
    pub owned: bool,
}

pub(crate) fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS external_open_requests(
        id TEXT PRIMARY KEY, target_key TEXT NOT NULL, data TEXT,
        received INTEGER NOT NULL DEFAULT 0, created INTEGER NOT NULL);",
    )?;
    Ok(())
}
fn prune(conn: &Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM external_open_requests WHERE created < ?1",
        [chrono::Utc::now().timestamp() - 86400],
    )?;
    conn.execute("DELETE FROM external_open_requests WHERE data IS NULL AND rowid NOT IN (SELECT rowid FROM external_open_requests ORDER BY rowid DESC LIMIT 64)", [])?;
    Ok(())
}
impl Store {
    /// All inputs are validated before the transaction, so a malformed launch
    /// cannot partially submit earlier arguments.
    pub fn submit_external_opens(&self, targets: &[Target]) -> Result<Vec<Submission>> {
        ensure!(
            !targets.is_empty() && targets.len() <= MAX_PENDING,
            "Open at most eight torrents at a time"
        );
        for target in targets {
            target.validate()?;
        }
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        prune(&tx)?;
        let mut result = Vec::new();
        for target in targets {
            let key = target.key()?;
            if let Some(id) = tx.query_row("SELECT id FROM external_open_requests WHERE target_key=?1 AND data IS NOT NULL", [&key], |r| r.get(0)).optional()? {
                result.push(Submission { id, owned: false });
                continue;
            }
            let count: i64 = tx.query_row(
                "SELECT count(*) FROM external_open_requests WHERE data IS NOT NULL",
                [],
                |r| r.get(0),
            )?;
            ensure!(
                count < MAX_PENDING as i64,
                "Review the pending torrents in Download Engine first"
            );
            let id = uuid::Uuid::new_v4().to_string();
            tx.execute("INSERT INTO external_open_requests(id,target_key,data,created) VALUES(?1,?2,?3,?4)", params![id,key,serde_json::to_string(target)?,chrono::Utc::now().timestamp()])?;
            result.push(Submission { id, owned: true });
        }
        tx.commit()?;
        Ok(result)
    }
    /// Only the desktop calls this. Acknowledge even while a dialog is open;
    /// requests remain durable until that review is accepted or dismissed.
    pub fn receive_external_opens(&self) -> Result<Inbox> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        prune(&tx)?;
        let newly_received = tx.execute(
            "UPDATE external_open_requests SET received=1 WHERE received=0 AND data IS NOT NULL",
            [],
        )?;
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM external_open_requests WHERE data IS NOT NULL",
            [],
            |r| r.get(0),
        )?;
        let row: Option<(String,String)> = tx.query_row("SELECT id,substr(data,1,196609) FROM external_open_requests WHERE data IS NOT NULL ORDER BY rowid LIMIT 1", [], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        let next = if let Some((id, data)) = row {
            match (|| -> Result<Target> {
                ensure!(data.len() <= MAX_TEXT * 6, "External request is too large");
                let target: Target = serde_json::from_str(&data)?;
                target.validate()?;
                Ok(target)
            })() {
                Ok(target) => Some(Pending { id, target }),
                Err(error) => {
                    // Report corruption once and keep later valid requests usable.
                    tx.execute(
                        "UPDATE external_open_requests SET data=NULL WHERE id=?1",
                        [id],
                    )?;
                    tx.commit()?;
                    return Err(error.context("An invalid external torrent request was discarded"));
                }
            }
        } else {
            None
        };
        tx.commit()?;
        Ok(Inbox {
            next,
            count: usize::try_from(count)?,
            newly_received,
        })
    }
    pub fn external_opens_received(&self, submitted: &[Submission]) -> Result<bool> {
        let conn = self.connect()?;
        for request in submitted {
            let received: Option<bool> = conn
                .query_row(
                    "SELECT received FROM external_open_requests WHERE id=?1",
                    [&request.id],
                    |r| r.get(0),
                )
                .optional()?;
            if received != Some(true) {
                return Ok(false);
            }
        }
        Ok(true)
    }
    /// On handoff timeout withdraw only this launch's unreceived requests.
    /// If the desktop won the race, its acknowledged requests are retained.
    pub fn withdraw_unreceived_external_opens(&self, submitted: &[Submission]) -> Result<bool> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut all_received = true;
        for request in submitted {
            let received: Option<bool> = tx
                .query_row(
                    "SELECT received FROM external_open_requests WHERE id=?1",
                    [&request.id],
                    |r| r.get(0),
                )
                .optional()?;
            all_received &= received == Some(true);
            if request.owned && received == Some(false) {
                tx.execute(
                    "DELETE FROM external_open_requests WHERE id=?1 AND received=0",
                    [&request.id],
                )?;
            }
        }
        tx.commit()?;
        Ok(all_received)
    }
    pub fn dismiss_external_open(&self, id: &str) -> Result<()> {
        self.connect()?.execute(
            "UPDATE external_open_requests SET data=NULL WHERE id=?1",
            [id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
