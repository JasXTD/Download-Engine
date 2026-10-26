use anyhow::{Context, Result};
use chrono::{Local, NaiveDateTime, TimeZone};
use std::{collections::HashSet, path::Path};

use crate::{i18n::tr, model::Job, queue_schedule::RecurringSchedule};

pub const MAIN_QUEUE: &str = "main";
/// Hard global cap, also the largest per-queue limit accepted from UI/CLI/MCP.
pub const MAX_ACTIVE: usize = 32;
/// Most links one batch accepts, pasted or generated from a pattern.
pub const MAX_BATCH_LINKS: usize = 1000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadQueue {
    pub id: String,
    pub name: String,
    pub max_active: usize,
    pub running: bool,
    pub scheduled_at: Option<i64>,
    pub recurring: Option<RecurringSchedule>,
    pub synchronize: bool,
}

impl DownloadQueue {
    pub fn has_schedule(&self) -> bool {
        self.scheduled_at.is_some() || self.recurring.is_some()
    }

    pub fn automatic(&self) -> bool {
        self.recurring
            .as_ref()
            .is_some_and(|schedule| schedule.enabled)
    }

    pub fn state_label(&self, now: i64) -> String {
        if let Some(at) = self.scheduled_at {
            return format!("Starts {}", schedule_label(at));
        }
        let state = if self.running {
            "Queue running"
        } else {
            "Queue paused"
        };
        let Some(schedule) = &self.recurring else {
            return state.into();
        };
        if !schedule.enabled {
            return format!("{state} · Schedule suspended");
        }
        match schedule.active_window(now) {
            Ok(Some(window)) => format!("Running · pauses {}", schedule_label(window.stop)),
            Ok(None) => match schedule.next_window(&Local, now) {
                Ok(Some(window)) => format!("Waiting · starts {}", schedule_label(window.start)),
                Ok(None) => "Waiting for a valid local time".into(),
                Err(error) => format!("Schedule error: {error}"),
            },
            Err(error) => format!("Schedule error: {error}"),
        }
    }
}

pub fn queue_name(value: &str) -> Result<&str> {
    let name = value.trim();
    anyhow::ensure!(!name.is_empty(), "Enter a queue name");
    anyhow::ensure!(
        name.chars().count() <= 64 && !name.chars().any(char::is_control),
        "Use a queue name of at most 64 characters without control characters"
    );
    Ok(name)
}

pub fn batch_jobs(text: &str, directory: &Path) -> Result<Vec<Job>> {
    anyhow::ensure!(text.len() <= 1_000_000, "The batch is too large");
    let mut jobs = Vec::new();
    let mut urls = HashSet::new();
    for (line, value) in text.lines().enumerate() {
        if value.trim().is_empty() {
            continue;
        }
        let job = Job::new(
            value.trim(),
            None,
            directory.to_path_buf(),
            uuid::Uuid::new_v4().to_string(),
        )
        .with_context(|| format!("Line {} is invalid", line + 1))?;
        if urls.insert(job.url.clone()) {
            jobs.push(job);
        }
        anyhow::ensure!(
            jobs.len() <= MAX_BATCH_LINKS,
            "Add at most 1,000 URLs per batch"
        );
    }
    anyhow::ensure!(!jobs.is_empty(), "Paste at least one HTTP or HTTPS URL");
    Ok(jobs)
}

/// Zero padding for numbered links.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Padding {
    /// As many digits as the longer of From and To as typed: 1 to 100 gives 001…100.
    #[default]
    Auto,
    /// At least this many digits; 0 adds no zeros.
    Width(u8),
}

/// Numbered or lettered links generated from one `*` placeholder, such as
/// `https://example.com/img*.jpg` from 1 to 100.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkPattern {
    prefix: String,
    suffix: String,
    first: u64,
    count: usize,
    /// Digits for numbers, `None` for letters.
    width: Option<usize>,
}

impl LinkPattern {
    pub fn new(pattern: &str, from: &str, to: &str, padding: Padding) -> Result<Self> {
        let (prefix, suffix) = pattern
            .trim()
            .split_once('*')
            .context(tr("The link pattern needs one *."))?;
        anyhow::ensure!(
            !suffix.contains('*'),
            tr("Use only one * in the link pattern.")
        );
        // Two probes reveal a placeholder in the scheme, login, site, port or #fragment.
        if let (Ok(a), Ok(b)) = (
            reqwest::Url::parse(&format!("{prefix}0{suffix}")),
            reqwest::Url::parse(&format!("{prefix}1{suffix}")),
        ) {
            let site = |url: &reqwest::Url| {
                (
                    url.scheme().to_owned(),
                    url.username().to_owned(),
                    url.password().map(str::to_owned),
                    url.host_str().map(str::to_owned),
                    url.port(),
                )
            };
            anyhow::ensure!(
                site(&a) == site(&b) && (a.path() != b.path() || a.query() != b.query()),
                tr("Put the * in the path or query part of the link.")
            );
        }
        let (from, to) = (from.trim(), to.trim());
        let (first, last, width) = if let (Some(first), Some(last)) = (number(from), number(to)) {
            let width = match padding {
                Padding::Auto => from.len().max(to.len()),
                Padding::Width(width) => width.into(),
            };
            (first, last, Some(width))
        } else if let (Some(first), Some(last)) = (letter(from), letter(to))
            && first.is_ascii_lowercase() == last.is_ascii_lowercase()
        {
            (first.into(), last.into(), None)
        } else {
            anyhow::bail!(tr(
                "Use whole numbers, or single letters a–z, for From and To."
            ));
        };
        anyhow::ensure!(first <= last, tr("From must not be greater than To."));
        // Checked before generating anything, so a huge range costs nothing.
        let span = last - first;
        anyhow::ensure!(
            span < MAX_BATCH_LINKS as u64,
            tf!(
                "This pattern makes {} links. Add at most 1,000 per batch.",
                u128::from(span) + 1
            )
        );
        let links = Self {
            prefix: prefix.to_owned(),
            suffix: suffix.to_owned(),
            first,
            count: span as usize + 1,
            width,
        };
        crate::model::validate_download_url(&links.first())?;
        Ok(links)
    }

    /// Number of links, from 1 to [`MAX_BATCH_LINKS`].
    pub fn count(&self) -> usize {
        self.count
    }

    /// Whether From and To are letters, where zero padding does not apply.
    pub fn letters(&self) -> bool {
        self.width.is_none()
    }

    pub fn first(&self) -> String {
        self.link(0)
    }

    pub fn last(&self) -> String {
        self.link(self.count - 1)
    }

    pub fn links(&self) -> impl Iterator<Item = String> + '_ {
        (0..self.count).map(|index| self.link(index))
    }

    fn link(&self, index: usize) -> String {
        let value = self.first + index as u64;
        let (prefix, suffix) = (&self.prefix, &self.suffix);
        match self.width {
            Some(width) => format!("{prefix}{value:0width$}{suffix}"),
            None => format!("{prefix}{}{suffix}", char::from(value as u8)),
        }
    }
}

fn number(value: &str) -> Option<u64> {
    (!value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()))
        .then(|| value.parse().ok())
        .flatten()
}

fn letter(value: &str) -> Option<u8> {
    match value.as_bytes() {
        [c] if c.is_ascii_alphabetic() => Some(*c),
        _ => None,
    }
}

pub fn local_schedule(date: &str, time: &str, now: i64) -> Result<i64> {
    let value = NaiveDateTime::parse_from_str(
        &format!("{} {}", date.trim(), time.trim()),
        "%Y-%m-%d %H:%M",
    )
    .context("Use date YYYY-MM-DD and time HH:MM (24-hour clock)")?;
    let instant = Local.from_local_datetime(&value).single()
        .context("That local time is ambiguous or unavailable because of a clock change; choose another time")?;
    anyhow::ensure!(instant.timestamp() > now, "Choose a time in the future");
    Ok(instant.timestamp())
}

pub fn schedule_label(at: i64) -> String {
    Local
        .timestamp_opt(at, 0)
        .single()
        .map(|value| value.format("%b %d, %H:%M").to_string())
        .unwrap_or_else(|| "Invalid scheduled time".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHOTOS: &str = "https://example.com/photos/img*.jpg";

    fn links(pattern: &LinkPattern) -> Vec<String> {
        pattern.links().collect()
    }

    fn error(pattern: &str, from: &str, to: &str) -> String {
        LinkPattern::new(pattern, from, to, Padding::Auto)
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn link_patterns_pad_numbers_automatically_or_to_a_chosen_width() {
        let auto = LinkPattern::new(PHOTOS, "1", "100", Padding::Auto).unwrap();
        assert_eq!(auto.count(), 100);
        assert!(!auto.letters());
        assert_eq!(auto.first(), "https://example.com/photos/img001.jpg");
        assert_eq!(auto.last(), "https://example.com/photos/img100.jpg");
        let all = links(&auto);
        assert_eq!(all.len(), 100);
        assert_eq!(all[9], "https://example.com/photos/img010.jpg");
        assert_eq!(all.iter().collect::<HashSet<_>>().len(), 100);
        // Auto follows the longer bound as typed, so leading zeros in From count.
        let typed = LinkPattern::new(PHOTOS, " 001 ", "5", Padding::Auto).unwrap();
        assert_eq!(typed.last(), "https://example.com/photos/img005.jpg");
        let off = LinkPattern::new(PHOTOS, "8", "11", Padding::Width(0)).unwrap();
        assert_eq!(
            links(&off),
            ["img8.jpg", "img9.jpg", "img10.jpg", "img11.jpg"]
                .map(|name| format!("https://example.com/photos/{name}"))
        );
        let wide = LinkPattern::new(PHOTOS, "9", "10", Padding::Width(4)).unwrap();
        assert_eq!(wide.first(), "https://example.com/photos/img0009.jpg");
        assert_eq!(wide.last(), "https://example.com/photos/img0010.jpg");
        // Numbers longer than the chosen width are never cut.
        let narrow = LinkPattern::new(PHOTOS, "998", "1000", Padding::Width(2)).unwrap();
        assert_eq!(narrow.first(), "https://example.com/photos/img998.jpg");
        assert_eq!(narrow.last(), "https://example.com/photos/img1000.jpg");
    }

    #[test]
    fn link_patterns_accept_letters_queries_ftp_and_a_single_item() {
        let lower = LinkPattern::new(
            "https://example.com/part-*.zip",
            "a",
            "e",
            Padding::Width(3),
        )
        .unwrap();
        assert!(lower.letters());
        assert_eq!(
            links(&lower),
            ["a", "b", "c", "d", "e"].map(|c| format!("https://example.com/part-{c}.zip"))
        );
        let upper =
            LinkPattern::new("https://example.com/disk*.iso", "X", "Z", Padding::Auto).unwrap();
        assert_eq!(upper.first(), "https://example.com/diskX.iso");
        assert_eq!(upper.last(), "https://example.com/diskZ.iso");
        let query = LinkPattern::new(
            "https://example.com/get?page=*&size=large",
            "0",
            "2",
            Padding::Auto,
        )
        .unwrap();
        assert_eq!(query.last(), "https://example.com/get?page=2&size=large");
        let ftp =
            LinkPattern::new("ftp://example.com/pub/part*.bin", "1", "2", Padding::Auto).unwrap();
        assert_eq!(ftp.first(), "ftp://example.com/pub/part1.bin");
        let single = LinkPattern::new(PHOTOS, "7", "7", Padding::Auto).unwrap();
        assert_eq!(single.count(), 1);
        assert_eq!(single.first(), single.last());
        assert_eq!(links(&single), ["https://example.com/photos/img7.jpg"]);
    }

    #[test]
    fn link_patterns_stop_at_the_batch_limit_before_generating() {
        let full = LinkPattern::new(PHOTOS, "1", "1000", Padding::Auto).unwrap();
        assert_eq!(full.count(), MAX_BATCH_LINKS);
        assert_eq!(links(&full).len(), MAX_BATCH_LINKS);
        assert_eq!(
            error(PHOTOS, "1", "1001"),
            "This pattern makes 1001 links. Add at most 1,000 per batch."
        );
        // The full u64 range is reported without overflow or generating links.
        assert_eq!(
            error(PHOTOS, "0", &u64::MAX.to_string()),
            "This pattern makes 18446744073709551616 links. Add at most 1,000 per batch."
        );
        let generated = links(&full).join("\n");
        let jobs = batch_jobs(&generated, &std::env::temp_dir()).unwrap();
        assert_eq!(jobs.len(), MAX_BATCH_LINKS);
        assert_eq!(jobs[0].name, "img0001.jpg");
        assert_eq!(jobs[999].url, "https://example.com/photos/img1000.jpg");
    }

    #[test]
    fn link_patterns_reject_bad_placeholders_ranges_and_links() {
        let placement = "Put the * in the path or query part of the link.";
        for (pattern, message) in [
            (
                "https://example.com/photos/img.jpg",
                "The link pattern needs one *.",
            ),
            ("", "The link pattern needs one *."),
            (
                "https://example.com/*/img*.jpg",
                "Use only one * in the link pattern.",
            ),
            ("https://cdn*.example.com/a.jpg", placement),
            ("http*://example.com/a.jpg", placement),
            ("https://example.com:8*/a.jpg", placement),
            ("https://user*@example.com/a.jpg", placement),
            ("https://example.com/a.jpg#part*", placement),
        ] {
            assert_eq!(error(pattern, "1", "3"), message, "{pattern}");
        }
        for (from, to) in [
            ("", "5"),
            ("1", ""),
            ("x1", "5"),
            ("-1", "5"),
            ("1.5", "5"),
            ("1", "z"),
            ("a", "Z"),
            ("aa", "zz"),
            ("é", "ü"),
            ("1", "99999999999999999999"),
        ] {
            assert_eq!(
                error(PHOTOS, from, to),
                "Use whole numbers, or single letters a–z, for From and To.",
                "{from}..{to}"
            );
        }
        for (from, to) in [("10", "1"), ("z", "a")] {
            assert_eq!(error(PHOTOS, from, to), "From must not be greater than To.");
        }
        assert!(error("example.com/img*.jpg", "1", "2").contains("http://"));
        for pattern in ["file:///C:/img*.jpg", "sftp://example.com/img*.jpg"] {
            assert!(LinkPattern::new(pattern, "1", "2", Padding::Auto).is_err());
        }
    }

    #[test]
    fn link_pattern_errors_follow_the_interface_language() {
        let _german = crate::i18n::scope(crate::i18n::Language::German);
        assert_eq!(
            error(PHOTOS, "10", "1"),
            "„Von“ darf nicht größer als „Bis“ sein."
        );
    }
}
