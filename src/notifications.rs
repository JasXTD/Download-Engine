//! Notify only durable terminal attempts, not historical entries or retry waits.
use crate::{
    i18n::tr,
    model::{Job, Status},
    preferences::Preferences,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    pub job: String,
    pub status: Status,
}
impl Finished {
    pub fn from_job(job: &Job) -> Option<Self> {
        matches!(job.status, Status::Complete | Status::Failed).then(|| Self {
            job: job.id.clone(),
            status: job.status,
        })
    }
    /// Whether this is the job's current terminal attempt, and if so whether it
    /// failed. A synchronization check that found the file up to date is not a download.
    pub fn outcome(&self, job: &Job) -> Option<bool> {
        if job.status == Status::Complete
            && job
                .synchronization
                .as_ref()
                .is_some_and(|state| state.phase == crate::synchronization::Phase::UpToDate)
        {
            return None;
        }
        if self.job != job.id
            || self.status != job.status
            || !matches!(self.status, Status::Complete | Status::Failed)
        {
            return None;
        }
        Some(job.status == Status::Failed)
    }
    pub fn notice(&self, job: &Job, preferences: Preferences) -> Option<Notice> {
        let failed = self.outcome(job)?;
        if (failed && !preferences.notify_failed) || (!failed && !preferences.notify_complete) {
            return None;
        }
        Some(Notice {
            queue: None,
            job: job.id.clone(),
            name: job.name.clone(),
            completed: u32::from(!failed),
            failed: u32::from(failed),
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub queue: Option<String>,
    pub job: String,
    pub name: String,
    pub completed: u32,
    pub failed: u32,
}
impl Notice {
    pub fn merge(&mut self, next: Self) {
        self.completed = self.completed.saturating_add(next.completed);
        self.failed = self.failed.saturating_add(next.failed);
        self.job = next.job;
        self.name = next.name;
    }
    pub fn title(&self) -> String {
        if self.queue.is_some() {
            return if self.completed <= 1 {
                tr("Queue complete").into()
            } else {
                tf!("{} queues complete", self.completed)
            };
        }
        if self.completed.saturating_add(self.failed) == 1 {
            if self.failed > 0 {
                tr("Download needs attention").into()
            } else {
                tr("Download complete").into()
            }
        } else {
            tf!(
                "{} completed, {} need attention",
                self.completed,
                self.failed
            )
        }
    }
    pub fn text(&self) -> String {
        let latest = self.completed.saturating_add(self.failed) > 1;
        let instruction = match (self.queue.is_some(), latest) {
            (true, true) => "Click to review the latest queue.",
            (true, false) => "Click to review this queue.",
            (false, true) => "Click to review the latest download.",
            (false, false) => "Click to review this download.",
        };
        format!("{}\n{}", self.name, tr(instruction))
    }
}
/// A copied file link waiting for review. It never merges with download notices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkOffer {
    pub name: String,
    pub host: String,
}
impl LinkOffer {
    pub fn title(&self) -> String {
        tr("Download copied link?").into()
    }
    pub fn text(&self) -> String {
        format!(
            "{} · {}\n{}",
            self.name,
            self.host,
            tr("Click to review it in Download Engine.")
        )
    }
}
/// A message a script asked to show with xd.notify. Clicking it opens the run's log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptNotice {
    pub run: String,
    pub script: String,
    pub title: String,
    pub text: String,
}
impl ScriptNotice {
    pub fn text(&self) -> String {
        let footer = format!("{} · {}", self.script, tr("Click to open its run log."));
        if self.text.is_empty() {
            footer
        } else {
            format!("{}\n{footer}", self.text)
        }
    }
}
/// Script notices wait in order; older ones beyond this many are dropped.
const MAX_PENDING_SCRIPTS: usize = 20;

/// A bounded batch behind the currently visible notification. Do not replace
/// its job identity when another download completes while the user is reading it.
#[derive(Default)]
pub struct Notices {
    pub active: Option<Notice>,
    pending: Option<Notice>,
    pending_queue: Option<Notice>,
    active_link: Option<LinkOffer>,
    pending_link: Option<LinkOffer>,
    active_script: Option<ScriptNotice>,
    pending_scripts: std::collections::VecDeque<ScriptNotice>,
}
impl Notices {
    /// Only the newest copied link waits; an older one is replaced.
    pub fn push_link(&mut self, link: LinkOffer) {
        self.pending_link = Some(link);
    }
    /// A copied link is shown before waiting download notices: it answers what
    /// the user just did. Only one notification is shown at a time.
    pub fn next_link(&mut self) -> Option<&LinkOffer> {
        if self.showing() {
            return None;
        }
        self.active_link = self.pending_link.take();
        self.active_link.as_ref()
    }
    /// Whether the notification that was clicked or closed offered a copied link.
    pub fn dismiss_link(&mut self) -> bool {
        self.active_link.take().is_some()
    }
    pub fn push(&mut self, notice: Notice) {
        if notice.queue.is_some() {
            if let Some(pending) = &mut self.pending_queue {
                pending.queue.clone_from(&notice.queue);
                pending.merge(notice);
            } else {
                self.pending_queue = Some(notice);
            }
            return;
        }
        if let Some(pending) = &mut self.pending {
            pending.merge(notice);
        } else {
            self.pending = Some(notice);
        }
    }
    pub fn next_notice(&mut self) -> Option<&Notice> {
        if self.showing() {
            return None;
        }
        self.active = self.pending.take().or_else(|| self.pending_queue.take());
        self.active.as_ref()
    }
    pub fn dismiss(&mut self) -> Option<Notice> {
        self.active.take()
    }
    /// Script messages never merge; each waits its turn after download notices.
    pub fn push_script(&mut self, notice: ScriptNotice) {
        if self.pending_scripts.len() == MAX_PENDING_SCRIPTS {
            self.pending_scripts.pop_front();
        }
        self.pending_scripts.push_back(notice);
    }
    pub fn next_script(&mut self) -> Option<&ScriptNotice> {
        if self.showing() || self.pending.is_some() || self.pending_queue.is_some() {
            return None;
        }
        self.active_script = self.pending_scripts.pop_front();
        self.active_script.as_ref()
    }
    pub fn dismiss_script(&mut self) -> Option<ScriptNotice> {
        self.active_script.take()
    }
    fn showing(&self) -> bool {
        self.active.is_some() || self.active_link.is_some() || self.active_script.is_some()
    }
    pub fn clear(&mut self) {
        self.active = None;
        self.pending = None;
        self.pending_queue = None;
        self.active_link = None;
        self.pending_link = None;
        self.active_script = None;
        self.pending_scripts.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn job(status: Status) -> Job {
        let mut job = Job::new(
            "https://example.com/file.zip?secret=hidden",
            None,
            std::env::temp_dir(),
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
        job.status = status;
        job.error = Some("secret server error".into());
        job
    }
    #[test]
    fn only_current_terminal_attempts_notify_and_preferences_apply() {
        let mut item = job(Status::Complete);
        let event = Finished::from_job(&item).unwrap();
        let defaults = Preferences::default();
        let notice = event.notice(&item, defaults).unwrap();
        assert_eq!(notice.title(), "Download complete");
        assert!(!notice.text().contains("secret"));
        assert!(
            event
                .notice(
                    &item,
                    Preferences {
                        notify_complete: false,
                        ..defaults
                    }
                )
                .is_none()
        );
        item.status = Status::Queued;
        assert!(event.notice(&item, defaults).is_none());
        for status in [Status::Queued, Status::Downloading, Status::Paused] {
            item.status = status;
            assert!(Finished::from_job(&item).is_none());
            assert!(
                Finished {
                    job: item.id.clone(),
                    status
                }
                .notice(&item, defaults)
                .is_none()
            );
        }
        item.status = Status::Failed;
        let failed = Finished::from_job(&item).unwrap();
        assert_eq!(
            failed.notice(&item, defaults).unwrap().title(),
            "Download needs attention"
        );
        assert!(
            failed
                .notice(
                    &item,
                    Preferences {
                        notify_failed: false,
                        ..defaults
                    }
                )
                .is_none()
        );
        item.id = "another job".into();
        assert!(failed.notice(&item, defaults).is_none());
    }
    #[test]
    fn outcomes_ignore_notification_settings_but_not_stale_or_up_to_date_events() {
        let mut item = job(Status::Complete);
        let done = Finished::from_job(&item).unwrap();
        assert_eq!(done.outcome(&item), Some(false));
        item.status = Status::Failed;
        assert_eq!(done.outcome(&item), None, "A later attempt replaced it");
        assert_eq!(
            Finished::from_job(&item).unwrap().outcome(&item),
            Some(true)
        );
        item.status = Status::Complete;
        item.synchronization = Some(crate::synchronization::State {
            phase: crate::synchronization::Phase::UpToDate,
            previous: crate::synchronization::CompletedCopy {
                path: std::env::temp_dir().join("file.zip"),
                downloaded: 0,
                total: None,
                plan: None,
                sha256: None,
                remote: None,
                expected_mime: None,
            },
            checked_at: None,
        });
        assert_eq!(done.outcome(&item), None);
    }
    #[test]
    fn bursts_coalesce_without_replacing_the_active_click_target() {
        let first = job(Status::Complete);
        let mut notices = Notices::default();
        notices.push(
            Finished::from_job(&first)
                .unwrap()
                .notice(&first, Preferences::default())
                .unwrap(),
        );
        assert_eq!(notices.next_notice().unwrap().job, first.id);
        let mut last = String::new();
        for i in 0..1000 {
            let next = job(if i % 2 == 0 {
                Status::Complete
            } else {
                Status::Failed
            });
            last.clone_from(&next.id);
            notices.push(
                Finished::from_job(&next)
                    .unwrap()
                    .notice(&next, Preferences::default())
                    .unwrap(),
            );
        }
        assert!(notices.next_notice().is_none());
        assert_eq!(notices.dismiss().unwrap().job, first.id);
        let batch = notices.next_notice().unwrap();
        assert_eq!(batch.job, last);
        assert_eq!(batch.title(), "500 completed, 500 need attention");
        assert!(batch.text().contains("latest"));
        notices.clear();
        assert!(notices.dismiss().is_none());
        assert!(notices.next_notice().is_none());
    }
}

#[cfg(test)]
mod queue_tests {
    use super::*;
    #[test]
    fn queue_balloons_stay_distinct_from_downloads_and_coalesce_with_correct_click_target() {
        let mut notices = Notices::default();
        notices.push(Notice {
            queue: None,
            job: "file".into(),
            name: "file.zip".into(),
            completed: 1,
            failed: 0,
        });
        assert_eq!(notices.next_notice().unwrap().title(), "Download complete");
        for id in ["one", "two"] {
            notices.push(Notice {
                queue: Some(id.into()),
                job: String::new(),
                name: id.into(),
                completed: 1,
                failed: 0,
            });
        }
        assert_eq!(notices.dismiss().unwrap().job, "file");
        let queue = notices.next_notice().unwrap();
        assert_eq!(queue.title(), "2 queues complete");
        assert_eq!(queue.queue.as_deref(), Some("two"));
        assert!(queue.text().contains("latest"));
        notices.clear();
        assert!(notices.next_notice().is_none());
    }
}

#[cfg(test)]
mod link_tests {
    use super::*;
    fn link(name: &str) -> LinkOffer {
        LinkOffer {
            name: name.into(),
            host: "downloads.example.com".into(),
        }
    }
    #[test]
    fn copied_links_keep_only_the_newest_and_never_take_a_download_click_target() {
        let mut notices = Notices::default();
        notices.push(Notice {
            queue: None,
            job: "file".into(),
            name: "file.zip".into(),
            completed: 1,
            failed: 0,
        });
        notices.push_link(link("old.zip"));
        notices.push_link(link("new.zip"));
        // A copied link answers what the user just did and is shown first.
        let shown = notices.next_link().unwrap().clone();
        assert_eq!(shown, link("new.zip"));
        assert_eq!(shown.title(), "Download copied link?");
        assert_eq!(
            shown.text(),
            "new.zip · downloads.example.com\nClick to review it in Download Engine."
        );
        assert!(notices.next_notice().is_none());
        assert!(notices.next_link().is_none());
        assert!(notices.dismiss_link());
        assert!(notices.dismiss().is_none());
        // The replaced older link is gone; the download notice keeps its target.
        assert!(notices.next_link().is_none());
        assert_eq!(notices.next_notice().unwrap().job, "file");
        notices.push_link(link("later.zip"));
        assert!(notices.next_link().is_none(), "One notification at a time");
        assert!(!notices.dismiss_link());
        assert_eq!(notices.dismiss().unwrap().job, "file");
        assert_eq!(notices.next_link().unwrap().name, "later.zip");
        notices.clear();
        assert!(!notices.dismiss_link());
        assert!(notices.next_link().is_none());
    }
}

#[cfg(test)]
mod script_tests {
    use super::*;
    fn script(title: &str) -> ScriptNotice {
        ScriptNotice {
            run: format!("run-{title}"),
            script: "Retry failed downloads".into(),
            title: title.into(),
            text: "Holiday photos.zip still needs attention.".into(),
        }
    }
    #[test]
    fn script_notices_wait_their_turn_in_order_and_open_their_own_run() {
        let mut notices = Notices::default();
        notices.push(Notice {
            queue: None,
            job: "file".into(),
            name: "file.zip".into(),
            completed: 1,
            failed: 0,
        });
        notices.push_script(script("first"));
        notices.push_script(script("second"));
        // Download notices go first; a script message never merges with them.
        assert!(notices.next_script().is_none());
        assert_eq!(notices.next_notice().unwrap().job, "file");
        assert!(
            notices.next_script().is_none(),
            "One notification at a time"
        );
        assert_eq!(notices.dismiss().unwrap().job, "file");
        let shown = notices.next_script().unwrap().clone();
        assert_eq!(shown.title, "first");
        assert_eq!(
            shown.text(),
            "Holiday photos.zip still needs attention.\nRetry failed downloads · Click to open its run log."
        );
        assert!(notices.next_link().is_none());
        assert_eq!(notices.dismiss_script().unwrap().run, "run-first");
        assert_eq!(notices.next_script().unwrap().title, "second");
        notices.dismiss_script();
        let empty = ScriptNotice {
            text: String::new(),
            ..script("bare")
        };
        assert_eq!(
            empty.text(),
            "Retry failed downloads · Click to open its run log."
        );
        // Only the newest 20 wait; older ones are dropped.
        for i in 0..25 {
            notices.push_script(script(&i.to_string()));
        }
        assert_eq!(notices.next_script().unwrap().title, "5");
        notices.clear();
        assert!(notices.dismiss_script().is_none());
        assert!(notices.next_script().is_none());
    }
}
