//! Browser batches are atomic and acknowledge the same receipt after a lost reply.
use crate::{categories::Category, model::Job, queue_store::enqueue_batch_in, store::Store};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Destination {
    Automatic,
    Downloads,
    Documents,
    Programs,
    Compressed,
    Music,
    Video,
    Other,
}
impl Destination {
    fn category(self) -> Option<Category> {
        match self {
            Self::Documents => Some(Category::Documents),
            Self::Programs => Some(Category::Programs),
            Self::Compressed => Some(Category::Compressed),
            Self::Music => Some(Category::Music),
            Self::Video => Some(Category::Video),
            Self::Other => Some(Category::Other),
            Self::Automatic | Self::Downloads => None,
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub url: String,
    #[serde(default)]
    pub filename: Option<String>,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct QueueChoice {
    pub id: String,
    pub name: String,
    pub running: bool,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct FolderChoice {
    pub id: Destination,
    pub name: String,
    pub path: Option<String>,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct Options {
    pub queues: Vec<QueueChoice>,
    pub destinations: Vec<FolderChoice>,
}

pub(crate) fn initialize(conn: &Connection) -> Result<()> {
    // A digest identifies the request without retaining signed URLs or filenames
    // in a second history after the user removes their jobs.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS browser_batches(
        request_id TEXT PRIMARY KEY, fingerprint BLOB NOT NULL CHECK(length(fingerprint)=32),
        job_ids TEXT NOT NULL);",
    )?;
    Ok(())
}

impl Store {
    pub fn browser_batch_options(&self) -> Result<Options> {
        let rules = self.download_folders()?;
        let mut destinations = vec![
            FolderChoice {
                id: Destination::Automatic,
                name: "Automatic folders".into(),
                path: None,
            },
            FolderChoice {
                id: Destination::Downloads,
                name: "Default downloads".into(),
                path: Some(self.paths.downloads.display().to_string()),
            },
        ];
        for id in [
            Destination::Documents,
            Destination::Programs,
            Destination::Compressed,
            Destination::Music,
            Destination::Video,
            Destination::Other,
        ] {
            let category = id.category().context("Missing folder category")?;
            destinations.push(FolderChoice {
                id,
                name: category.label().into(),
                path: Some(
                    rules
                        .category_folder(&self.paths.downloads, category)
                        .display()
                        .to_string(),
                ),
            });
        }
        let queues = self
            .queues()?
            .into_iter()
            .map(|queue| QueueChoice {
                id: queue.id,
                name: queue.name,
                running: queue.running,
            })
            .collect();
        Ok(Options {
            queues,
            destinations,
        })
    }

    pub fn enqueue_browser_batch(
        &self,
        request_id: &str,
        links: &[Link],
        queue: &str,
        destination: Destination,
    ) -> Result<Vec<String>> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ids =
            self.enqueue_browser_batch_in(&tx, request_id, links, None, queue, destination)?;
        tx.commit()?;
        Ok(ids)
    }

    /// Allows a reviewed grabber selection and its removal from the pending
    /// preview to commit together with the usual batch receipt and downloads.
    /// `page` carries the validated source page and its public-only restriction.
    pub(crate) fn enqueue_browser_batch_in(
        &self,
        conn: &Connection,
        request_id: &str,
        links: &[Link],
        page: Option<(&str, bool)>,
        queue: &str,
        destination: Destination,
    ) -> Result<Vec<String>> {
        let token = request_id
            .strip_prefix("batch:")
            .context("Invalid batch identifier")?;
        let uuid = uuid::Uuid::parse_str(token).context("Invalid batch identifier")?;
        ensure!(
            request_id == format!("batch:{uuid}"),
            "Invalid batch identifier"
        );
        ensure!(
            !links.is_empty() && links.len() <= 1000,
            "Select between 1 and 1,000 links"
        );
        ensure!(
            !queue.is_empty() && queue.len() <= 64,
            "Invalid queue identifier"
        );
        let mut seen = HashSet::new();
        let mut jobs = Vec::new();
        for (index, link) in links.iter().enumerate() {
            crate::model::validate_url(&link.url)?;
            ensure!(
                !crate::hls::is_playlist_url(&link.url),
                "HLS / DASH videos need a quality preview. Use Preview video for each stream."
            );
            ensure!(
                link.filename.as_ref().is_none_or(|name| name.len() <= 1024),
                "A filename is too long"
            );
            let mut job = Job::new(
                &link.url,
                link.filename.as_deref(),
                self.paths.downloads.clone(),
                format!("{request_id}:{index}"),
            )?;
            job.source_page = page.map(|(page, _)| page.to_owned());
            job.public_only = page.is_some_and(|(_, public_only)| public_only);
            if seen.insert(job.url.clone()) {
                jobs.push(job);
            }
        }
        let canonical: Vec<_> = jobs.iter().map(|job| (&job.url, &job.name)).collect();
        let encoded = if page.is_some_and(|(_, public_only)| public_only) {
            serde_json::to_vec(&(queue, destination, canonical, "public_only"))?
        } else {
            // Preserve receipt compatibility for existing browser batches.
            serde_json::to_vec(&(queue, destination, canonical))?
        };
        ensure!(
            encoded.len() <= 900_000,
            "The selected links are too large for one batch"
        );
        let fingerprint = Sha256::digest(&encoded);
        let receipt: Option<(Vec<u8>, String)> = conn
            .query_row(
                "SELECT fingerprint,job_ids FROM browser_batches WHERE request_id=?1",
                [request_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((previous, ids)) = receipt {
            ensure!(
                previous.as_slice() == &fingerprint[..],
                "This batch identifier belongs to a different selection"
            );
            return Ok(serde_json::from_str(&ids)?);
        }
        // Resolve destinations from local settings, never from paths supplied by
        // a page or extension. A retry keeps the original jobs and destinations.
        let rules = self.download_folders()?;
        for job in &mut jobs {
            job.directory = match destination {
                Destination::Automatic => rules.destination(&self.paths.downloads, &job.name),
                Destination::Downloads => self.paths.downloads.clone(),
                category => rules.category_folder(
                    &self.paths.downloads,
                    category.category().context("Missing category")?,
                ),
            };
        }
        let saved = enqueue_batch_in(conn, &jobs, queue)?;
        let ids: Vec<_> = saved.into_iter().map(|job| job.id).collect();
        conn.execute(
            "INSERT INTO browser_batches(request_id,fingerprint,job_ids) VALUES(?1,?2,?3)",
            params![request_id, &fingerprint[..], serde_json::to_string(&ids)?],
        )?;
        Ok(ids)
    }
}
