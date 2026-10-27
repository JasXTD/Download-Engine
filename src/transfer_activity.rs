//! Ephemeral worker activity, shared by Store clones in the owning desktop.
//! It is deliberately separate from durable recovery state. A worker guard
//! removes its activity on success, cancellation, errors, and unwinding.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Connecting,
    CheckingSaved,
    Downloading,
    Assembling,
    Merging,
    Verifying,
    Saving,
}
impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Connecting => "Connecting",
            Self::CheckingSaved => "Checking saved progress",
            Self::Downloading => "Downloading",
            Self::Assembling => "Assembling and verifying file",
            Self::Merging => "Merging video",
            Self::Verifying => "Verifying file",
            Self::Saving => "Saving file",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentState {
    Waiting,
    SendingGet,
    ReceivingData,
    Verifying,
    Completed,
}
impl SegmentState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Waiting => "Waiting for connection…",
            Self::SendingGet => "Send GET…",
            Self::ReceivingData => "Receiving data…",
            Self::Verifying => "Checking segment…",
            Self::Completed => "Completed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SegmentSnapshot {
    pub index: usize,
    pub start: u64,
    pub end: u64,
    pub downloaded: u64,
    pub state: SegmentState,
}

/// Media parts have their own URLs and often unknown sizes; they are not byte
/// ranges of the final merged file. Each active worker publishes its own part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaSnapshot {
    pub index: usize,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub state: SegmentState,
}

#[derive(Clone)]
struct Entry {
    phase: Phase,
    segments: Vec<SegmentSnapshot>,
    media: Vec<MediaSnapshot>,
}

#[derive(Clone, Default)]
pub struct Activity(Arc<Mutex<HashMap<String, Entry>>>);
impl Activity {
    fn entries(&self) -> MutexGuard<'_, HashMap<String, Entry>> {
        // Only display data lives here; no user callback or I/O runs under this
        // lock. Recovering a poisoned display cache cannot affect saved data.
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    pub fn get(&self, id: &str) -> Option<Phase> {
        self.entries().get(id).map(|entry| entry.phase)
    }
    pub fn segments(&self, id: &str) -> Vec<SegmentSnapshot> {
        let mut list = self
            .entries()
            .get(id)
            .map(|entry| entry.segments.clone())
            .unwrap_or_default();
        list.sort_unstable_by_key(|segment| segment.start);
        list
    }
    pub(crate) fn begin(&self, id: &str) -> Guard {
        self.entries().insert(
            id.into(),
            Entry {
                phase: Phase::Connecting,
                segments: Vec::new(),
                media: Vec::new(),
            },
        );
        Guard {
            activity: self.clone(),
            id: id.into(),
        }
    }
    pub fn seed_preview(&self, id: &str, phase: Phase, segments: Vec<SegmentSnapshot>) {
        self.entries().insert(
            id.into(),
            Entry {
                phase,
                segments,
                media: Vec::new(),
            },
        );
    }
    pub fn media(&self, id: &str) -> Option<MediaSnapshot> {
        self.media_parts(id).into_iter().next()
    }
    pub fn media_parts(&self, id: &str) -> Vec<MediaSnapshot> {
        let mut parts = self
            .entries()
            .get(id)
            .map(|entry| entry.media.clone())
            .unwrap_or_default();
        parts.sort_unstable_by_key(|part| part.index);
        parts
    }
    #[cfg(feature = "screenshot")]
    pub fn seed_media_preview(&self, id: &str, media: Vec<MediaSnapshot>) {
        self.seed_preview(id, Phase::Downloading, Vec::new());
        for part in media {
            self.set_media(id, part);
        }
    }
    pub(crate) fn set_media(&self, id: &str, media: MediaSnapshot) {
        if let Some(entry) = self.entries().get_mut(id) {
            if let Some(previous) = entry
                .media
                .iter_mut()
                .find(|part| part.index == media.index)
            {
                *previous = media;
            } else {
                entry.media.push(media);
            }
        }
    }
    pub(crate) fn remove_media(&self, id: &str, index: usize) {
        if let Some(entry) = self.entries().get_mut(id) {
            entry.media.retain(|part| part.index != index);
        }
    }
    pub(crate) fn set(&self, id: &str, phase: Phase) {
        if let Some(entry) = self.entries().get_mut(id) {
            entry.phase = phase;
        }
    }
    pub(crate) fn set_segments(&self, id: &str, segments: Vec<SegmentSnapshot>) {
        if let Some(entry) = self.entries().get_mut(id) {
            entry.segments = segments;
        }
    }
    pub(crate) fn update_segment(
        &self,
        id: &str,
        index: usize,
        downloaded: u64,
        state: SegmentState,
    ) {
        if let Some(entry) = self.entries().get_mut(id)
            && let Some(segment) = entry.segments.iter_mut().find(|s| s.index == index)
        {
            segment.downloaded = downloaded;
            segment.state = state;
        }
    }
}
pub(crate) struct Guard {
    activity: Activity,
    id: String,
}
impl Drop for Guard {
    fn drop(&mut self) {
        self.activity.entries().remove(&self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parallel_media_updates_are_independent_and_removed_after_checkpoint() {
        let activity = Activity::default();
        let guard = activity.begin("video");
        for index in [3, 1, 2] {
            activity.set_media(
                "video",
                MediaSnapshot {
                    index,
                    downloaded: 10,
                    total: Some(100),
                    state: SegmentState::ReceivingData,
                },
            );
        }
        activity.set_media(
            "video",
            MediaSnapshot {
                index: 2,
                downloaded: 100,
                total: Some(100),
                state: SegmentState::Verifying,
            },
        );
        let parts = activity.media_parts("video");
        assert_eq!(
            parts.iter().map(|part| part.index).collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(parts[0].downloaded, 10);
        assert_eq!(parts[1].downloaded, 100);
        activity.remove_media("video", 2);
        assert_eq!(activity.media_parts("video").len(), 2);
        drop(guard);
        assert!(activity.media_parts("video").is_empty());
    }
    #[test]
    fn clones_share_activity_and_worker_exit_clears_it() {
        let activity = Activity::default();
        let clone = activity.clone();
        let guard = activity.begin("job");
        assert_eq!(clone.get("job"), Some(Phase::Connecting));
        clone.set("job", Phase::Downloading);
        clone.set_segments(
            "job",
            vec![SegmentSnapshot {
                index: 0,
                start: 0,
                end: 99,
                downloaded: 25,
                state: SegmentState::ReceivingData,
            }],
        );
        assert_eq!(activity.get("job"), Some(Phase::Downloading));
        assert_eq!(activity.segments("job").len(), 1);
        drop(guard);
        clone.set("job", Phase::Saving);
        assert_eq!(activity.get("job"), None);
        assert!(activity.segments("job").is_empty());
    }
}
