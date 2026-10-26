//! Browser media handoffs only create a bounded, expiring desktop review.
use crate::{
    browser_batch::{Destination, Link},
    model,
    store::Store,
};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    File,
    Hls,
    Dash,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub url: String,
    pub filename: String,
    pub label: String,
    pub kind: Kind,
    pub size: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Review {
    /// Set by site readers; preserved in the pending review until selection.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub public_only: bool,
    pub page_url: String,
    pub page_title: String,
    pub items: Vec<Item>,
    pub truncated: bool,
}

#[derive(Clone, Debug)]
pub struct Pending {
    pub id: String,
    pub review: Review,
}

fn clean_text(value: &str) -> String {
    value
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(c,
        '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .take(200)
        .collect::<String>()
        .trim()
        .to_owned()
}

impl Review {
    pub(crate) fn normalized(&self) -> Result<Self> {
        ensure!(self.items.len() <= 1000, "Too many videos in this review");
        ensure!(self.page_title.len() <= 2048, "The page title is too long");
        let mut review = self.clone();
        review.page_url = model::validate_url(&self.page_url)?.to_string();
        review.page_title = clean_text(&self.page_title);
        let mut seen = HashSet::new();
        for item in &mut review.items {
            item.url = model::validate_url(&item.url)?.to_string();
            ensure!(
                seen.insert(item.url.clone()),
                "Duplicate video in this review"
            );
            ensure!(
                item.filename.len() <= 1024 && item.label.len() <= 2048,
                "Video details are too long"
            );
            ensure!(
                item.size
                    .is_none_or(|size| size > 0 && size <= 9_007_199_254_740_991),
                "Invalid video size"
            );
            ensure!(
                item.kind != Kind::File || !crate::hls::is_playlist_url(&item.url),
                "Video streams need a quality preview"
            );
            item.filename = model::safe_filename(&item.filename);
            item.label = clean_text(&item.label);
        }
        ensure!(
            serde_json::to_vec(&review)?.len() <= 850_000,
            "This video review is too large"
        );
        Ok(review)
    }
}

fn token(id: &str) -> Result<&str> {
    let token = id
        .strip_prefix("video:")
        .context("Invalid video review identifier")?;
    model::validate_job_id(token)?;
    Ok(token)
}

pub(crate) fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS browser_video_reviews(
        request_id TEXT PRIMARY KEY, fingerprint BLOB NOT NULL CHECK(length(fingerprint)=32),
        data TEXT, job_ids TEXT, created INTEGER NOT NULL);",
    )?;
    Ok(())
}

fn prune(conn: &Connection) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    // Keep only a digest after review/expiry, so receipt retries need no URLs.
    conn.execute(
        "DELETE FROM browser_video_reviews WHERE created < ?1",
        [now - 86400],
    )?;
    conn.execute(
        "UPDATE browser_video_reviews SET data=NULL WHERE data IS NOT NULL AND created < ?1",
        [now - 1800],
    )?;
    Ok(())
}

impl Store {
    pub fn request_browser_video_review(&self, id: &str, review: &Review) -> Result<bool> {
        token(id)?;
        let data = serde_json::to_string(&review.normalized()?)?;
        let fingerprint = Sha256::digest(data.as_bytes());
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        prune(&tx)?;
        let existing: Option<(Vec<u8>, bool)> = tx.query_row(
            "SELECT fingerprint,data IS NOT NULL FROM browser_video_reviews WHERE request_id=?1",
            [id], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
        if let Some((previous, pending)) = existing {
            ensure!(
                previous.as_slice() == &fingerprint[..],
                "Video review identifier belongs to another selection"
            );
            tx.commit()?;
            return Ok(pending);
        }
        let pending: i64 = tx.query_row(
            "SELECT count(*) FROM browser_video_reviews WHERE data IS NOT NULL",
            [],
            |row| row.get(0),
        )?;
        ensure!(
            pending < 10,
            "Review pending videos in Download Engine first"
        );
        tx.execute("INSERT INTO browser_video_reviews(request_id,fingerprint,data,created) VALUES(?1,?2,?3,?4)",
            params![id,&fingerprint[..],data,chrono::Utc::now().timestamp()])?;
        tx.commit()?;
        Ok(true)
    }

    pub fn next_browser_video_review(&self) -> Result<Option<Pending>> {
        let conn = self.connect()?;
        prune(&conn)?;
        let row: Option<(String,String)> = conn.query_row(
            "SELECT request_id,data FROM browser_video_reviews WHERE data IS NOT NULL ORDER BY created,rowid LIMIT 1", [],
            |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
        row.map(|(id, data)| {
            token(&id)?;
            let review: Review = serde_json::from_str(&data)?;
            Ok(Pending {
                id,
                review: review.normalized()?,
            })
        })
        .transpose()
    }

    pub fn dismiss_browser_video_review(&self, id: &str) -> Result<()> {
        token(id)?;
        self.connect()?.execute(
            "UPDATE browser_video_reviews SET data=NULL WHERE request_id=?1",
            [id],
        )?;
        Ok(())
    }

    pub fn take_browser_video_stream(&self, id: &str, index: usize) -> Result<String> {
        token(id)?;
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        prune(&tx)?;
        let data: String = tx
            .query_row(
                "SELECT data FROM browser_video_reviews WHERE request_id=?1 AND data IS NOT NULL",
                [id],
                |row| row.get(0),
            )
            .optional()?
            .context("This video review expired. Find videos again from the browser.")?;
        let review: Review = serde_json::from_str(&data)?;
        let review = review.normalized()?;
        let item = review.items.get(index).context("Invalid video selection")?;
        ensure!(
            item.kind != Kind::File,
            "Select a video stream to choose its quality"
        );
        let url = item.url.clone();
        tx.execute(
            "UPDATE browser_video_reviews SET data=NULL WHERE request_id=?1",
            [id],
        )?;
        tx.commit()?;
        Ok(url)
    }

    /// Queue only the explicitly selected direct files and consume their review
    /// in the same transaction. HLS/DASH must use the desktop quality picker.
    pub fn add_browser_video_selection(
        &self,
        id: &str,
        indices: &[usize],
        queue: &str,
        destination: Destination,
    ) -> Result<Vec<String>> {
        let batch = format!("batch:{}", token(id)?);
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        prune(&tx)?;
        let (data, receipt): (Option<String>, Option<String>) = tx
            .query_row(
                "SELECT data,job_ids FROM browser_video_reviews WHERE request_id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .context("This video review is no longer available")?;
        if let Some(receipt) = receipt {
            return Ok(serde_json::from_str(&receipt)?);
        }
        let review: Review = serde_json::from_str(
            &data.context("This video review expired. Find videos again from the browser.")?,
        )?;
        let review = review.normalized()?;
        let mut selected = HashSet::new();
        ensure!(
            !indices.is_empty() && indices.len() <= review.items.len(),
            "Select video files to download"
        );
        let links = indices
            .iter()
            .map(|index| {
                ensure!(selected.insert(*index), "Duplicate video selection");
                let item = review
                    .items
                    .get(*index)
                    .context("Invalid video selection")?;
                ensure!(
                    item.kind == Kind::File,
                    "Choose a quality before downloading a stream"
                );
                Ok(Link {
                    url: item.url.clone(),
                    filename: Some(item.filename.clone()),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        // Video servers often check that requests come from their own pages.
        let ids = self.enqueue_browser_batch_in(
            &tx,
            &batch,
            &links,
            Some((&review.page_url, review.public_only)),
            queue,
            destination,
        )?;
        tx.execute(
            "UPDATE browser_video_reviews SET data=NULL,job_ids=?2 WHERE request_id=?1",
            params![id, serde_json::to_string(&ids)?],
        )?;
        tx.commit()?;
        Ok(ids)
    }
}
