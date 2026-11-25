//! File ownership recorded while the completed torrent is still in private staging.
use anyhow::Result;
use de_torrent::storage::file_identity;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{File, Metadata},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) type Receipts = BTreeMap<PathBuf, FileReceipt>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FileReceipt {
    id: String,
    pub(super) size: u64,
    modified_ns: i128,
    #[cfg(unix)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content_hash: Option<[u8; 32]>,
}

impl FileReceipt {
    pub(crate) fn read(file: &File) -> Result<Self> {
        Self::from_metadata(file, &file.metadata()?)
    }

    pub(super) fn from_metadata(file: &File, metadata: &Metadata) -> Result<Self> {
        Ok(Self {
            id: file_identity(file)?,
            size: metadata.len(),
            modified_ns: timestamp_ns(metadata.modified()?),
            // Without a birth time the filesystem can recycle the recorded
            // inode. Read by offset so relocation can keep using its open file.
            #[cfg(unix)]
            content_hash: if metadata.created().is_ok_and(|birth| birth > UNIX_EPOCH) {
                None
            } else {
                Some(hash_contents(file, metadata.len())?)
            },
        })
    }

    pub(crate) fn matches(&self, file: &File) -> Result<bool> {
        let metadata = file.metadata()?;
        if metadata.len() != self.size
            || timestamp_ns(metadata.modified()?) != self.modified_ns
            || file_identity(file)? != self.id
        {
            return Ok(false);
        }
        #[cfg(unix)]
        if let Some(expected) = self.content_hash {
            return Ok(hash_contents(file, self.size)? == expected);
        }
        Ok(true)
    }
}

fn timestamp_ns(time: SystemTime) -> i128 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => after.as_nanos() as i128,
        Err(before) => -(before.duration().as_nanos() as i128),
    }
}

#[cfg(unix)]
fn hash_contents(file: &File, size: u64) -> Result<[u8; 32]> {
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::FileExt;

    let mut hash = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    let mut offset = 0;
    while offset < size {
        let count = (size - offset).min(buffer.len() as u64) as usize;
        file.read_exact_at(&mut buffer[..count], offset)?;
        hash.update(&buffer[..count]);
        offset += count as u64;
    }
    Ok(hash.finalize().into())
}

#[cfg(all(test, unix))]
#[test]
fn receipt_fingerprint_covers_every_chunk_and_preserves_the_copy_cursor() {
    use sha2::{Digest, Sha256};
    use std::io::{Seek, SeekFrom, Write};

    let mut file = tempfile::tempfile().unwrap();
    let mut bytes = vec![0x61; 70_000];
    *bytes.last_mut().unwrap() = 0x62;
    file.write_all(&bytes).unwrap();
    file.seek(SeekFrom::Start(31)).unwrap();
    let expected: [u8; 32] = Sha256::digest(&bytes).into();
    assert_eq!(hash_contents(&file, bytes.len() as u64).unwrap(), expected);
    assert_eq!(file.stream_position().unwrap(), 31);
    let modified = file.metadata().unwrap().modified().unwrap();
    let mut receipt = FileReceipt::read(&file).unwrap();
    receipt.content_hash = Some(expected);
    assert!(receipt.matches(&file).unwrap());
    file.write_all(b"z").unwrap();
    file.set_modified(modified).unwrap();
    assert!(!receipt.matches(&file).unwrap());
}

pub(crate) fn load(conn: &Connection, job: &str) -> Result<Option<Receipts>> {
    let json: Option<String> = conn
        .query_row(
            "SELECT files FROM torrent_file_receipts WHERE job_id=?1",
            [job],
            |r| r.get(0),
        )
        .optional()?;
    Ok(json.map(|data| serde_json::from_str(&data)).transpose()?)
}

pub(crate) fn save(tx: &Transaction<'_>, job: &str, files: &Receipts) -> Result<()> {
    tx.execute(
        "INSERT INTO torrent_file_receipts(job_id,files) VALUES(?1,?2)
         ON CONFLICT(job_id) DO UPDATE SET files=excluded.files",
        params![job, serde_json::to_string(files)?],
    )?;
    tx.execute(
        "UPDATE profile_format SET data_version=MAX(data_version,?1),
        min_writer_version=MAX(min_writer_version,?1) WHERE id=1",
        [crate::store::migrations::TORRENT_RECEIPT_VERSION],
    )?;
    Ok(())
}
