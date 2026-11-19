//! Space checks are estimates. The filesystem remains authoritative at each write.
use crate::{download_file, model::Job, store::Store};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{fmt, io, path::Path};

pub(crate) use de_torrent::storage::available_space as available;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Location {
    Temporary,
    Destination,
    SystemTemp,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Requirement {
    pub location: Location,
    pub required: u64,
    pub available: u64,
}

impl Location {
    fn folder(self, store: &Store, job: &Job) -> std::path::PathBuf {
        match self {
            Self::Temporary => store
                .paths
                .parts(&job.id)
                .unwrap_or_else(|_| store.paths.data.join("parts")),
            Self::Destination => job.directory.clone(),
            Self::SystemTemp => std::env::temp_dir(),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Temporary => "Download cache",
            Self::Destination => "Destination folder",
            Self::SystemTemp => "Temporary folder",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Wait {
    pub requirements: Vec<Requirement>,
    pub check_after: i64,
}

impl fmt::Display for Wait {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Waiting for disk space")
    }
}
impl std::error::Error for Wait {}

impl Wait {
    fn new(requirements: Vec<Requirement>) -> Self {
        Self {
            requirements,
            check_after: chrono::Utc::now().timestamp_millis().saturating_add(30_000),
        }
    }

    pub(crate) fn describe(&self, store: &Store, job: &Job) -> String {
        let mut message = crate::i18n::tr("Waiting for disk space").to_owned();
        for r in &self.requirements {
            let path = r.location.folder(store, job);
            message.push('\n');
            message.push_str(&tf!(
                "{} ({}) · {} bytes required, {} available",
                crate::i18n::tr(r.location.label()),
                path.display(),
                r.required,
                r.available
            ));
        }
        message
    }

    pub(crate) fn ready(&self, store: &Store, job: &Job, now: i64) -> bool {
        now >= self.check_after
            && !self.requirements.is_empty()
            && self.requirements.iter().all(|r| {
                available(&r.location.folder(store, job)).is_ok_and(|free| free >= r.required)
            })
    }

    pub(crate) fn from_error(store: &Store, job: &Job, error: &anyhow::Error) -> Option<Self> {
        if let Some(wait) = error.downcast_ref::<Self>() {
            return Some(wait.clone());
        }
        if let Some(wait) = error.chain().find_map(|e| e.downcast_ref::<Self>()) {
            return Some(wait.clone());
        }
        let helper_full = error
            .downcast_ref::<crate::torrent::HelperError>()
            .is_some_and(|e| e.code.as_deref() == Some(de_torrent::reason::DISK_FULL));
        if !helper_full
            && !error.chain().any(|e| {
                e.downcast_ref::<io::Error>()
                    .is_some_and(|e| e.kind() == io::ErrorKind::StorageFull)
            })
        {
            return None;
        }
        use crate::transfer_activity::Phase;
        let phase = store.activity.get(&job.id);
        let publishing = matches!(phase, Some(Phase::Assembling | Phase::Saving));
        let location = if publishing || job.torrent.is_some() {
            Location::Destination
        } else {
            Location::Temporary
        };
        let needed = match phase {
            // A finished HTTP stream at the hard-link step only needs room for metadata.
            Some(Phase::Saving) if job.received_sha256.is_some() => 0,
            Some(Phase::Assembling | Phase::Saving) => {
                job.downloaded.saturating_sub(staged_bytes(job))
            }
            _ => job.total.unwrap_or(0).saturating_sub(job.downloaded),
        };
        Some(Self::new(vec![Requirement {
            location,
            required: if needed == 0 { 256 * 1024 } else { needed },
            available: available(&location.folder(store, job)).unwrap_or(0),
        }]))
    }

    pub(crate) fn check_known(location: Location, required: u64, free: u64) -> Result<()> {
        if free < required {
            return Err(Self::new(vec![Requirement {
                location,
                required,
                available: free,
            }])
            .into());
        }
        Ok(())
    }
}

fn staged_bytes(job: &Job) -> u64 {
    let path = job.directory.join(format!(".idm-ex-{}.assembling", job.id));
    download_file::read_part_sync(&path)
        .ok()
        .and_then(|f| f.metadata().ok())
        .map_or(0, |m| m.len())
}

pub(crate) fn check(folder: &Path, location: Location, bytes: u64) -> Result<()> {
    if bytes > 0
        && let Ok(free) = available(folder)
    {
        return Wait::check_known(location, bytes, free);
    }
    Ok(())
}

pub(crate) fn check_transfer(
    job: &Job,
    root: &Path,
    total: u64,
    multiple_parts: bool,
) -> Result<()> {
    let remaining = total.saturating_sub(job.downloaded);
    let temporary = download_file::guard_download_parent(&root.join("part"))?;
    let destination = download_file::guard_download_parent(&job.directory.join(&job.name))?;
    let same_volume = download_file::same_volume(&temporary, &destination)?;
    let staged = if multiple_parts || !same_volume {
        staged_bytes(job)
    } else {
        0
    };
    let (cache, copy) = transfer_bytes(remaining, total, staged, same_volume, multiple_parts);
    let requirements: Vec<_> = [
        (root, Location::Temporary, cache),
        (job.directory.as_path(), Location::Destination, copy),
    ]
    .into_iter()
    .filter(|(_, _, required)| *required > 0)
    .filter_map(|(path, location, required)| {
        Some(Requirement {
            location,
            required,
            available: available(path).ok()?,
        })
    })
    .collect();
    if requirements.iter().any(|r| r.available < r.required) {
        return Err(Wait::new(requirements).into());
    }
    Ok(())
}

/// A direct rename consumes no second copy. During assembly the remaining parts
/// and destination copy must fit at the same time on their respective volumes.
fn transfer_bytes(
    remaining: u64,
    total: u64,
    staged: u64,
    same_volume: bool,
    multiple_parts: bool,
) -> (u64, u64) {
    let copy = total.saturating_sub(staged);
    match (same_volume, multiple_parts) {
        (true, false) => (remaining, 0),
        (true, true) => (remaining.saturating_add(copy), 0),
        (false, _) => (remaining, copy),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disk_full_publishing_a_received_stream_does_not_require_another_full_copy() {
        let (temp, store, _) = crate::torrent::completed_folder_fixture();
        let mut job = Job::new(
            "https://example.invalid/file",
            None,
            temp.path().to_owned(),
            "received".into(),
        )
        .unwrap();
        job.downloaded = 8 * 1024 * 1024;
        job.total = Some(job.downloaded);
        job.received_sha256 = Some("ab".repeat(32));
        let _active = store.activity.begin(&job.id);
        store
            .activity
            .set(&job.id, crate::transfer_activity::Phase::Saving);
        let error = io::Error::from(io::ErrorKind::StorageFull).into();
        let wait = Wait::from_error(&store, &job, &error).unwrap();
        assert_eq!(wait.requirements[0].location, Location::Destination);
        assert!((1..job.downloaded).contains(&wait.requirements[0].required));
    }
    #[test]
    fn disk_space_recognizes_helper_codes_and_ignores_unstructured_messages() {
        let (_temp, store, job) = crate::torrent::completed_folder_fixture();
        let error = anyhow::Error::new(crate::torrent::HelperError {
            code: Some(de_torrent::reason::DISK_FULL.into()),
            message: "Write failed".into(),
        });
        let wait = Wait::from_error(&store, &job, &error).unwrap();
        assert_eq!(wait.requirements[0].location, Location::Destination);
        assert!(Wait::from_error(&store, &job, &anyhow::anyhow!("disk_full")).is_none());
    }
    #[test]
    fn disk_space_accounts_for_reusable_parts_and_concurrent_destination_copy() {
        assert_eq!(transfer_bytes(30, 100, 0, true, false), (30, 0));
        assert_eq!(transfer_bytes(30, 100, 0, true, true), (130, 0));
        assert_eq!(transfer_bytes(30, 100, 0, false, false), (30, 100));
        assert_eq!(transfer_bytes(0, 100, 40, true, true), (60, 0));
        assert_eq!(transfer_bytes(0, 100, 40, false, true), (0, 60));
    }
}
