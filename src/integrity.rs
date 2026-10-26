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
    let mut file = download_file::read_completed(path)?;
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
    Ok((format!("{:x}", hasher.finalize()), file))
}
pub fn verify_completed(
    store: &Store,
    snapshot: &Job,
    expected: &str,
    cancel: &CancellationToken,
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
    let (actual, _guard) = hash_file(path, cancel)?;
    let mut conn = store.connect()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let data: String = tx
        .query_row("SELECT data FROM jobs WHERE id=?1", [&snapshot.id], |r| {
            r.get(0)
        })
        .context("This download has been removed from history")?;
    let mut current = crate::store::decode_job(&snapshot.id, &data)?;
    ensure!(
        current.status == Status::Complete && current.saved_path == snapshot.saved_path,
        "This download changed while its checksum was being calculated"
    );
    ensure!(!cancel.is_cancelled(), "Checksum calculation cancelled");
    current.integrity = State {
        expected,
        actual: Some(actual),
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
