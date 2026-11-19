//! SHA-256 describes bytes, not whether a download is safe to execute.
use crate::{
    download_file,
    model::{Job, Status},
    store::Store,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};
use tokio_util::sync::CancellationToken;
pub mod manifest;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub expected: Option<String>,
    pub actual: Option<String>,
}
impl State {
    pub fn is_default(&self) -> bool {
        self.expected.is_none() && self.actual.is_none()
    }
    pub fn matches(&self) -> Option<bool> {
        Some(
            self.expected
                .as_ref()?
                .eq_ignore_ascii_case(self.actual.as_ref()?),
        )
    }
    pub fn label(&self) -> &'static str {
        match self.matches() {
            Some(true) => "SHA-256 matched",
            Some(false) => "SHA-256 mismatch",
            None if self.actual.is_some() => "SHA-256 calculated",
            None if self.expected.is_some() => "SHA-256 pending",
            None => "SHA-256 not calculated",
        }
    }
    pub fn check(&self) -> Result<()> {
        if let Some(expected) = &self.expected {
            parse_expected(expected)?;
            ensure!(
                self.matches() == Some(true),
                "SHA-256 mismatch: the file does not match the expected checksum. Check the publisher's checksum before downloading again."
            );
        }
        Ok(())
    }
}
pub fn parse_expected(value: &str) -> Result<Option<String>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    ensure!(
        value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()),
        "Enter a SHA-256 checksum of exactly 64 hexadecimal characters"
    );
    Ok(Some(value.to_ascii_lowercase()))
}
/// Holds a read-only, non-reparse handle that denies writes/deletion on Windows
/// throughout hashing and the metadata commit. Cancellation never saves a partial hash.
pub fn hash_file(path: &Path, cancel: &CancellationToken) -> Result<(String, std::fs::File)> {
    let hash = HashedFile::read(path, cancel)?;
    #[cfg(unix)]
    hash.check_path(path)?;
    Ok((hash.digest, hash.file))
}

struct HashedFile {
    digest: String,
    file: std::fs::File,
    #[cfg(unix)]
    before: std::fs::Metadata,
}

impl HashedFile {
    fn read(path: &Path, cancel: &CancellationToken) -> Result<Self> {
        let mut file = download_file::read_completed(path)?;
        #[cfg(unix)]
        let before = file.metadata()?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0; 256 * 1024];
        loop {
            ensure!(!cancel.is_cancelled(), "Checksum calculation cancelled");
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        Ok(Self {
            digest: format!("{:x}", hasher.finalize()),
            file,
            #[cfg(unix)]
            before,
        })
    }

    #[cfg(unix)]
    fn check_path(&self, path: &Path) -> Result<()> {
        use std::os::unix::fs::MetadataExt;
        let stamp = |m: &std::fs::Metadata| {
            (
                m.dev(),
                m.ino(),
                m.len(),
                m.mtime(),
                m.mtime_nsec(),
                m.ctime(),
                m.ctime_nsec(),
            )
        };
        // An open descriptor pins the inode, but a sync client may replace the
        // directory entry or edit that inode while the hash is being calculated.
        let current = download_file::read_completed(path)
            .and_then(|f| Ok(f.metadata()?))
            .context("The saved file changed or became unavailable during checksum verification")?;
        ensure!(
            stamp(&self.before) == stamp(&current),
            "The saved file changed during checksum verification. Check it again."
        );
        Ok(())
    }
}
pub fn verify_completed(
    store: &Store,
    snapshot: &Job,
    expected: &str,
    cancel: &CancellationToken,
) -> Result<State> {
    verify_in_queue(store, snapshot, expected, cancel, None)
}

fn verify_in_queue(
    store: &Store,
    snapshot: &Job,
    expected: &str,
    cancel: &CancellationToken,
    queue: Option<&str>,
) -> Result<State> {
    ensure!(
        snapshot.status == Status::Complete,
        "Finish the download before checking its checksum"
    );
    let expected = parse_expected(expected)?;
    let path = snapshot
        .saved_path
        .as_ref()
        .context("The saved file location is unavailable")?;
    let hashed = HashedFile::read(path, cancel)?;
    record_verification(store, snapshot, expected, cancel, queue, hashed)
}

fn record_verification(
    store: &Store,
    snapshot: &Job,
    expected: Option<String>,
    cancel: &CancellationToken,
    queue: Option<&str>,
    hashed: HashedFile,
) -> Result<State> {
    let mut conn = store.connect()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let data: String = tx
        .query_row("SELECT data FROM jobs WHERE id=?1", [&snapshot.id], |r| {
            r.get(0)
        })
        .context("This download has been removed from history")?;
    let mut current = crate::store::decode_job(&snapshot.id, &data)?;
    if let Some(queue) = queue {
        let belongs: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM job_queues WHERE job_id=?1 AND queue_id=?2)",
            (&snapshot.id, queue),
            |r| r.get(0),
        )?;
        ensure!(
            belongs && current.integrity.expected == snapshot.integrity.expected,
            "The queue or expected checksum changed after preview"
        );
    }
    ensure!(
        current.status == Status::Complete && current.saved_path == snapshot.saved_path,
        "This download changed while its checksum was being calculated"
    );
    ensure!(!cancel.is_cancelled(), "Checksum calculation cancelled");
    #[cfg(unix)]
    hashed.check_path(
        current
            .saved_path
            .as_deref()
            .context("The saved file location is unavailable")?,
    )?;
    current.integrity = State {
        expected,
        actual: Some(hashed.digest),
    };
    tx.execute(
        "UPDATE jobs SET data=?1 WHERE id=?2",
        rusqlite::params![serde_json::to_string(&current)?, current.id],
    )?;
    tx.commit()?;
    Ok(current.integrity)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn changed_saved_file_never_receives_the_old_checksum() {
        use std::fs;
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(crate::paths::AppPaths {
            data: temp.path().join("profile"),
            downloads: temp.path().join("out"),
        })
        .unwrap();
        let path = temp.path().join("file.bin");
        let mut job = Job::new(
            "https://example.invalid/file.bin",
            None,
            temp.path().to_owned(),
            "checksum".into(),
        )
        .unwrap();
        job.status = Status::Complete;
        job.saved_path = Some(path.clone());
        store.enqueue(&job).unwrap();
        let token = CancellationToken::new();
        for change in ["replace", "edit", "remove"] {
            fs::write(&path, b"original").unwrap();
            // A distinct mtime also makes the same-size edit deterministic on coarse filesystems.
            fs::File::open(&path)
                .unwrap()
                .set_modified(std::time::SystemTime::UNIX_EPOCH)
                .unwrap();
            let hashed = HashedFile::read(&path, &token).unwrap();
            let expected = Some(hashed.digest.clone());
            match change {
                "replace" => {
                    let incoming = temp.path().join("synced");
                    fs::write(&incoming, b"new data").unwrap();
                    fs::rename(incoming, &path).unwrap();
                }
                "edit" => fs::write(&path, b"new data").unwrap(),
                _ => fs::remove_file(&path).unwrap(),
            }
            let error = record_verification(
                &store,
                &job,
                expected,
                &token,
                Some(crate::queue::MAIN_QUEUE),
                hashed,
            )
            .unwrap_err();
            assert!(
                error.to_string().contains("saved file changed"),
                "{change}: {error:#}"
            );
            assert_eq!(
                store.get(&job.id).unwrap().unwrap().integrity,
                State::default()
            );
        }
        fs::write(&path, b"abc").unwrap();
        let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(
            verify_completed(&store, &job, expected, &token)
                .unwrap()
                .matches(),
            Some(true)
        );
    }
    #[test]
    fn validates_and_normalizes_hashes_and_accepts_legacy_state() {
        assert_eq!(
            parse_expected(&"Ab".repeat(32)).unwrap(),
            Some("ab".repeat(32))
        );
        for bad in ["abc", &"g".repeat(64), &"a ".repeat(32)] {
            assert!(parse_expected(bad).is_err());
        }
        assert_eq!(parse_expected(" ").unwrap(), None);
        assert_eq!(
            serde_json::from_str::<State>("{}").unwrap(),
            State::default()
        );
    }
    #[test]
    fn known_hash_and_cancellation() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("abc");
        std::fs::write(&path, b"abc").unwrap();
        let cancel = CancellationToken::new();
        assert_eq!(
            hash_file(&path, &cancel).unwrap().0,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        cancel.cancel();
        assert!(hash_file(&path, &cancel).is_err());
    }
}
