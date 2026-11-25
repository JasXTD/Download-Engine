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
    nonce: Uuid,
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
                nonce,
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
        let registered = Self::load(&tx, job)?;
        let mut source: Option<(String, String)> = None;
        let mut upgrade = false;
        if let Some(stage) = &registered {
            let receipt = tx.query_row(
                "SELECT source_nonce, source_id FROM torrent_private_staging WHERE job_id=?1 AND directory_id=?2",
                params![job.id, stage.identity], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?))
            ).optional()?;
            upgrade = cfg!(windows) && receipt.is_none();
            source = receipt.and_then(|(nonce, id)| nonce.zip(id));
        }
        let (stage, directory) = match registered {
            Some(stage) if !upgrade => {
                let directory = Directory::open(&stage.path)
                    .context("Cannot reopen the torrent staging folder")?;
                directory.verify(&stage.identity)?;
                (stage, directory)
            }
            previous => {
                // Only the profile can indicate an old transfer. A precreated name in a
                // shared download folder must not make a new job read or import anything.
                let import_legacy = if let Some(old) = previous {
                    // An inherited Windows ACL can leave previously opened write
                    // handles active. Import authenticated pieces into a fresh
                    // directory instead of changing permissions on the live tree.
                    Directory::open(&old.path)?.verify(&old.identity)?;
                    source = Some((old.nonce.to_string(), old.identity));
                    old.import_legacy
                } else {
                    !chosen.is_empty()
                        && (job.downloaded > 0
                            || state_dir
                                .join(format!("{}.resume.json", hashes.key()))
                                .try_exists()?)
                };
                let nonce = Uuid::new_v4();
                let path = Self::path(job, nonce);
                // A crash before the transaction commits can leave an empty directory.
                // A new nonce on retry avoids adopting it, and no data is written yet.
                let directory = Directory::create_new(&path)
                    .context("Cannot create a private torrent staging folder")?;
                let identity = directory.identity()?;
                tx.execute("INSERT INTO torrent_staging(job_id, nonce, directory_id, import_legacy) VALUES(?1,?2,?3,?4)
                    ON CONFLICT(job_id) DO UPDATE SET nonce=excluded.nonce,directory_id=excluded.directory_id,published_id=NULL,import_legacy=excluded.import_legacy",
                    params![job.id, nonce.to_string(), identity, import_legacy])?;
                tx.execute(
                    "DELETE FROM torrent_file_receipts WHERE job_id=?1",
                    [&job.id],
                )?;
                tx.execute("INSERT OR REPLACE INTO torrent_private_staging(job_id,directory_id,source_nonce,source_id) VALUES(?1,?2,?3,?4)",
                    params![job.id, identity, source.as_ref().map(|s| &s.0), source.as_ref().map(|s| &s.1)])?;
                tx.execute("UPDATE profile_format SET data_version=MAX(data_version,?1),min_writer_version=MAX(min_writer_version,?1) WHERE id=1",
                    [crate::store::migrations::PRIVATE_STAGE_VERSION])?;
                (
                    Self {
                        nonce,
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
        if let Some((nonce, id)) = source {
            let previous = Self::path(job, Uuid::parse_str(&nonce)?);
            if !de_torrent::storage::import_verified_pieces(
                &state_dir,
                hashes,
                chosen,
                &previous,
                Some(&id),
                directory.clone(),
                cancel,
            )? {
                return Ok(None);
            }
            conn.execute("UPDATE torrent_private_staging SET source_nonce=NULL,source_id=NULL WHERE job_id=?1 AND directory_id=?2",
                params![job.id, stage.identity])?;
        }
        if stage.import_legacy {
            let legacy_path = job.directory.join(format!(".idm-ex-{}.torrent", job.id));
            if !de_torrent::storage::import_verified_pieces(
                &state_dir,
                hashes,
                chosen,
                &legacy_path,
                None,
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
