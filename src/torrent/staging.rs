//! The profile records which directory belongs to a job. A pathname alone is never
//! evidence of ownership, on first use, resume, cleanup, or after a process restart.
use super::{Job, Store};
use anyhow::{Context, Result};
use de_torrent::storage::Directory;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::{path::PathBuf, sync::Arc};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub(super) struct Stage {
    pub path: PathBuf,
    pub identity: String,
    pub published: Option<String>,
    import_legacy: bool,
}

impl Stage {
    pub fn load(conn: &Connection, job: &Job) -> Result<Option<Self>> {
        let row: Option<(String, String, Option<String>, bool)> = conn.query_row(
            "SELECT nonce, directory_id, published_id, import_legacy FROM torrent_staging WHERE job_id=?1",
            [&job.id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        ).optional()?;
        row.map(|(nonce, identity, published, import_legacy)| {
            let nonce = Uuid::parse_str(&nonce).context("Invalid saved torrent staging name")?;
            Ok(Self {
                path: Self::path(job, nonce),
                identity,
                published,
                import_legacy,
            })
        })
        .transpose()
    }

    fn path(job: &Job, nonce: Uuid) -> PathBuf {
        job.directory
            .join(format!(".idm-ex-{}-{nonce}.torrent", job.id))
    }

    pub fn open_for_move(&self) -> std::io::Result<Directory> {
        let directory = Directory::open_for_move(&self.path)?;
        directory.verify(&self.identity)?;
        Ok(directory)
    }

    /// Keep the host handle through the transfer. Pass its identity to the separate helper
    /// so it checks its own handle even if the path changed between the two opens.
    pub fn prepare(
        store: &Store,
        job: &Job,
        chosen: &[usize],
        cancel: &CancellationToken,
    ) -> Result<Option<(Self, Arc<Directory>)>> {
        let hashes = de_torrent::v2::InfoHashes::parse_magnet(&job.url)?;
        let mut conn = store.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state_dir = store.paths.data.join("torrents");
        let (stage, directory) = match Self::load(&tx, job)? {
            Some(stage) => {
                let directory = Directory::open(&stage.path)
                    .context("Cannot reopen the torrent staging folder")?;
                directory.verify(&stage.identity)?;
                (stage, directory)
            }
            None => {
                // Only the profile can indicate an old transfer. A precreated name in a
                // shared download folder must not make a new job read or import anything.
                let import_legacy = !chosen.is_empty()
                    && (job.downloaded > 0
                        || state_dir
                            .join(format!("{}.resume.json", hashes.key()))
                            .try_exists()?);
                let nonce = Uuid::new_v4();
                let path = Self::path(job, nonce);
                // A crash before the transaction commits can leave an empty directory.
                // A new nonce on retry avoids adopting it, and no data is written yet.
                let directory = Directory::create_new(&path)
                    .context("Cannot create a private torrent staging folder")?;
                let identity = directory.identity()?;
                tx.execute("INSERT INTO torrent_staging(job_id, nonce, directory_id, import_legacy) VALUES(?1,?2,?3,?4)",
                    params![job.id, nonce.to_string(), identity, import_legacy])?;
                // A writer that ignores the receipt would resume into the old staging name.
                tx.execute("UPDATE profile_format SET data_version=MAX(data_version,13),min_writer_version=MAX(min_writer_version,13) WHERE id=1", [])?;
                (
                    Self {
                        path,
                        identity,
                        published: None,
                        import_legacy,
                    },
                    directory,
                )
            }
        };
        tx.commit()?;
        let directory = Arc::new(directory);
        if stage.import_legacy {
            let legacy_path = job.directory.join(format!(".idm-ex-{}.torrent", job.id));
            if !de_torrent::storage::import_verified_pieces(
                &state_dir,
                hashes,
                chosen,
                &legacy_path,
                directory.clone(),
                cancel,
            )? {
                return Ok(None);
            }
            conn.execute(
                "UPDATE torrent_staging SET import_legacy=0 WHERE job_id=?1 AND directory_id=?2",
                params![job.id, stage.identity],
            )?;
        }
        Ok(Some((stage, directory)))
    }
}
