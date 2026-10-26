//! Explicit, one-shot queue actions. Arming is session-local, never restored on startup.
use crate::model::{Job, Status};
use anyhow::{Result, ensure};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Notify,
    Exit,
    Shutdown,
    Sleep,
    Hibernate,
}
impl Action {
    /// The choices in menu order, after Off.
    pub const ALL: [Self; 5] = [
        Self::Notify,
        Self::Exit,
        Self::Shutdown,
        Self::Sleep,
        Self::Hibernate,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Notify => "Notify",
            Self::Exit => "Exit",
            Self::Shutdown => "Shut down computer",
            Self::Sleep => "Sleep computer",
            Self::Hibernate => "Hibernate computer",
        }
    }
    /// Shut down, sleep and hibernate share one cancellable countdown and its safety checks.
    pub fn is_power(self) -> bool {
        matches!(self, Self::Shutdown | Self::Sleep | Self::Hibernate)
    }
}
/// Sleep and hibernation that Windows lets an app request on this computer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PowerStates {
    /// S1–S3. Apps cannot request Modern Standby (S0 low power idle).
    pub sleep: bool,
    /// S4 with a hibernation file present.
    pub hibernate: bool,
}
impl PowerStates {
    /// Why `action` cannot run on this computer, or `None` when it can.
    pub fn unavailable(self, action: Action) -> Option<&'static str> {
        match action {
            Action::Sleep if !self.sleep => Some(
                "Sleep is unavailable: Windows doesn't let apps put this computer to sleep, for example on Modern Standby PCs.",
            ),
            Action::Hibernate if !self.hibernate => Some(
                "Hibernate is off or not supported on this computer. An administrator can turn it on with powercfg /hibernate on.",
            ),
            _ => None,
        }
    }
}
/// The Windows sound for a finished batch of downloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    Complete,
    Attention,
}
/// Downloads that finished since the last batch sound. Files that finish while
/// others still run or wait to start share one sound.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SoundBatch {
    completed: u32,
    failed: u32,
}
impl SoundBatch {
    pub fn record(&mut self, failed: bool) {
        let count = if failed {
            &mut self.failed
        } else {
            &mut self.completed
        };
        *count = count.saturating_add(1);
    }
    pub fn is_empty(&self) -> bool {
        self.completed == 0 && self.failed == 0
    }
    /// Ends the batch once no download is running or waiting to start. Files
    /// that need attention use the warning sound only while those notifications are on.
    pub fn finish(&mut self, pending: bool, sound: bool, notify_failed: bool) -> Option<Sound> {
        if pending || self.is_empty() {
            return None;
        }
        let batch = std::mem::take(self);
        if !sound {
            None
        } else if batch.failed > 0 && notify_failed {
            Some(Sound::Attention)
        } else if batch.completed > 0 {
            Some(Sound::Complete)
        } else {
            None
        }
    }
}
/// Whether a download is transferring or starts without the user: queued in a
/// started queue, started immediately, or waiting for an automatic retry.
pub fn downloads_pending(store: &crate::store::Store) -> Result<bool> {
    Ok(store.connect()?.query_row(
        "SELECT EXISTS(SELECT 1 FROM jobs j
        LEFT JOIN job_queues m ON m.job_id=j.id
        LEFT JOIN download_queues q ON q.id=m.queue_id
        WHERE json_extract(j.data,'$.status')='downloading'
        OR (json_extract(j.data,'$.status')='queued' AND (q.running<>0
            OR j.id IN (SELECT job_id FROM immediate_downloads))))",
        [],
        |row| row.get(0),
    )?)
}
pub struct Watch {
    pub action: Action,
    pub name: String,
    pending: Vec<String>,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Progress {
    Waiting,
    Complete,
    Cancelled,
}
impl Watch {
    pub fn new(action: Action, name: String, jobs: &[Job]) -> Result<Self> {
        let pending: Vec<_> = jobs
            .iter()
            .filter(|j| j.status != Status::Complete)
            .map(|j| j.id.clone())
            .collect();
        ensure!(
            !pending.is_empty(),
            "Add unfinished downloads before enabling a completion action"
        );
        Ok(Self {
            action,
            name,
            pending,
        })
    }
    pub fn progress(&self, jobs: &[Job]) -> Progress {
        if self
            .pending
            .iter()
            .any(|id| !jobs.iter().any(|j| &j.id == id))
        {
            return Progress::Cancelled;
        }
        if !jobs.is_empty()
            && jobs
                .iter()
                .all(|j| j.status == Status::Complete && j.integrity.matches() != Some(false))
        {
            Progress::Complete
        } else {
            Progress::Waiting
        }
    }
}
pub fn can_exit(jobs: &[Job], editing: bool) -> bool {
    !editing
        && jobs
            .iter()
            .all(|j| j.status == Status::Complete && j.integrity.matches() != Some(false))
}
// Consult persisted work rather than the UI's last refresh. A completion script
// or CLI command can still be running after its triggering download finishes.
pub fn background_idle(store: &crate::store::Store) -> Result<bool> {
    Ok(store.connect()?.query_row(
        "SELECT NOT (EXISTS(SELECT 1 FROM script_runs WHERE state IN ('queued','running'))
        OR EXISTS(SELECT 1 FROM control_requests WHERE state IN ('pending','running'))
        OR EXISTS(SELECT 1 FROM grabber_schedules WHERE run_id IS NOT NULL))",
        [],
        |row| row.get(0),
    )?)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn job() -> Job {
        Job::new(
            "https://example.com/a",
            None,
            std::env::temp_dir(),
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap()
    }
    #[test]
    fn completion_requires_every_remaining_file_and_never_treats_removal_as_success() {
        let mut j = job();
        let watch = Watch::new(Action::Notify, "Main".into(), &[j.clone()]).unwrap();
        for state in [
            Status::Queued,
            Status::Downloading,
            Status::Paused,
            Status::Failed,
        ] {
            j.status = state;
            assert_eq!(watch.progress(&[j.clone()]), Progress::Waiting);
        }
        assert_eq!(watch.progress(&[]), Progress::Cancelled);
        j.status = Status::Complete;
        assert_eq!(watch.progress(&[j.clone()]), Progress::Complete);
        assert_eq!(watch.progress(&[j.clone(), job()]), Progress::Waiting);
        assert!(Watch::new(Action::Exit, "Main".into(), &[j.clone()]).is_err());
        assert!(can_exit(&[j.clone()], false));
        assert!(!can_exit(&[j.clone()], true));
        assert!(!can_exit(&[j.clone(), job()], false));
        j.integrity = crate::integrity::State {
            expected: Some("0".repeat(64)),
            actual: Some("1".repeat(64)),
        };
        assert_eq!(watch.progress(&[j.clone()]), Progress::Waiting);
        assert!(!can_exit(&[j], false));
    }
    #[test]
    fn power_choices_follow_windows_and_explain_why_they_are_unavailable() {
        assert_eq!(
            Action::ALL.map(Action::is_power),
            [false, false, true, true, true]
        );
        let none = PowerStates::default();
        let all = PowerStates {
            sleep: true,
            hibernate: true,
        };
        for action in [Action::Notify, Action::Exit, Action::Shutdown] {
            assert_eq!(none.unavailable(action), None);
        }
        assert!(
            none.unavailable(Action::Sleep)
                .unwrap()
                .contains("Modern Standby")
        );
        assert!(
            none.unavailable(Action::Hibernate)
                .unwrap()
                .contains("powercfg /hibernate on")
        );
        assert!(Action::ALL.iter().all(|&a| all.unavailable(a).is_none()));
        let sleep_only = PowerStates {
            hibernate: false,
            ..all
        };
        assert_eq!(sleep_only.unavailable(Action::Sleep), None);
        assert!(sleep_only.unavailable(Action::Hibernate).is_some());
    }
    #[test]
    fn one_sound_per_batch_waits_for_idle_and_failures_follow_their_notifications() {
        let mut batch = SoundBatch::default();
        assert_eq!(batch.finish(false, true, true), None, "Nothing finished");
        batch.record(false);
        batch.record(false);
        assert_eq!(batch.finish(true, true, true), None, "Others still running");
        assert!(!batch.is_empty());
        assert_eq!(batch.finish(false, true, true), Some(Sound::Complete));
        assert!(batch.is_empty());
        assert_eq!(batch.finish(false, true, true), None, "Plays once");
        batch.record(false);
        batch.record(true);
        assert_eq!(batch.finish(false, true, true), Some(Sound::Attention));
        batch.record(false);
        batch.record(true);
        assert_eq!(
            batch.finish(false, true, false),
            Some(Sound::Complete),
            "Failure alerts are off"
        );
        batch.record(true);
        assert_eq!(batch.finish(false, true, false), None);
        batch.record(false);
        assert_eq!(batch.finish(false, false, true), None, "Sound is off");
        assert!(batch.is_empty(), "A silent batch still ends");
    }
    #[test]
    fn pending_downloads_include_started_queues_retries_and_immediate_starts() {
        let root = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(crate::paths::AppPaths {
            data: root.path().join("data"),
            downloads: root.path().join("downloads"),
        })
        .unwrap();
        assert!(!downloads_pending(&store).unwrap());
        let mut j = store.enqueue(&job()).unwrap();
        // The main queue starts new downloads by default.
        assert!(downloads_pending(&store).unwrap());
        for (status, pending) in [
            (Status::Complete, false),
            (Status::Failed, false),
            (Status::Paused, false),
            (Status::Downloading, true),
        ] {
            j.status = status;
            store.save(&j).unwrap();
            assert_eq!(downloads_pending(&store).unwrap(), pending, "{status:?}");
        }
        j.status = Status::Queued;
        j.retry.next_at = Some(chrono::Utc::now().timestamp_millis() + 3_600_000);
        store.save(&j).unwrap();
        assert!(downloads_pending(&store).unwrap(), "Waiting for a retry");
        store
            .set_queue_running(crate::queue::MAIN_QUEUE, false)
            .unwrap();
        assert!(!downloads_pending(&store).unwrap(), "Stopped queue");
        store.start_immediately(&j.id).unwrap();
        assert!(downloads_pending(&store).unwrap(), "Start immediately");
        j = store.get(&j.id).unwrap().unwrap();
        j.status = Status::Downloading;
        store.save(&j).unwrap();
        assert!(downloads_pending(&store).unwrap());
    }
}
