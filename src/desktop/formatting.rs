//! Download labels, progress and readable sizes.
use super::*;

pub(super) fn display_queue_name<'a>(id: &str, name: &'a str) -> &'a str {
    if id == MAIN_QUEUE && name == "Main queue" {
        tr("Main queue")
    } else {
        name
    }
}

pub(super) fn display_schedule(schedule: &RecurringSchedule) -> String {
    let days = if schedule.days == EVERY_DAY {
        tr("Every day").to_owned()
    } else {
        DAY_NAMES
            .iter()
            .enumerate()
            .filter(|(index, _)| schedule.days & (1 << index) != 0)
            .map(|(_, day)| tr(day))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "{} · {}–{}{}",
        days,
        clock_label(schedule.start_minute),
        clock_label(schedule.stop_minute),
        if schedule.overnight() {
            tr(" (next day)")
        } else {
            ""
        }
    )
}

pub(super) fn short_queue_name(name: &str) -> String {
    if name.chars().count() > 19 {
        format!("{}…", name.chars().take(18).collect::<String>())
    } else {
        name.to_owned()
    }
}

pub(super) fn primary(label: &str, p: Palette) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).color(p.accent_text).strong())
        .fill(p.accent)
        .stroke(egui::Stroke::NONE)
}

pub(super) fn status_label(job: &Job, queue_running: bool, now: i64) -> String {
    if job.status == Status::Queued && !queue_running && job.retry.next_at.is_none() {
        return tr("Queue paused").into();
    }
    if job.status == Status::Downloading
        && job.hls.as_ref().is_some_and(|hls| {
            hls.playlist.audio.is_some() && hls.completed.len() == hls.playlist.part_count()
        })
    {
        return tr("Merging audio and video").into();
    }
    if job.status == Status::Queued && job.quota_waiting {
        return if queue_running {
            tr("Quota reached")
        } else {
            tr("Queue paused")
        }
        .into();
    }
    if job.status == Status::Queued
        && let Some(at) = job.retry.next_at
    {
        if !queue_running {
            return tr("Retry paused").into();
        }
        let seconds = at.saturating_sub(now).max(0) as u64;
        let seconds = seconds.div_ceil(1000);
        return if seconds == 0 {
            tr("Retry queued").into()
        } else if seconds < 60 {
            tf!("Retry in {seconds}s", seconds = seconds)
        } else if seconds < 3600 {
            tf!("Retry in {}m", seconds.div_ceil(60))
        } else {
            tf!("Retry in {}h", seconds.div_ceil(3600))
        };
    }
    if let Some(state) = &job.synchronization {
        tr(state.label(job.status)).into()
    } else if job.status == Status::Complete {
        tr("Completed").into()
    } else {
        tr(job.status.label()).into()
    }
}

pub(super) fn retry_description(job: &Job, queue_running: bool) -> Option<String> {
    (job.status == Status::Queued && job.retry.next_at.is_some()).then(|| {
        let suffix = if queue_running {
            tr("Pause to stop automatic retries.")
        } else {
            tr("Waiting for this queue to start.")
        };
        tf!(
            "Automatic retry {} of {}. {suffix}",
            job.retry.attempts,
            download_engine::retry::MAX_RETRIES,
            suffix = suffix
        )
    })
}

pub(super) fn supports_authentication(job: &Job) -> bool {
    job.hls.is_none()
        && job
            .ftp
            .as_ref()
            .is_none_or(|ftp| ftp.security != download_engine::ftp::Security::Plain)
}

pub(super) fn resume_action(job: &Job) -> &'static str {
    if job.plan.is_some() || job.hls.is_some() || job.ftp.is_some() {
        tr("Resume")
    } else {
        tr("Start again")
    }
}

pub(super) fn resume_description(job: &Job) -> &'static str {
    if job.status == Status::Complete {
        tr("Download finished")
    } else if job.hls.is_some() {
        tr("Resumes from the last complete video segment")
    } else if job.ftp.is_some() {
        tr("Verifies saved bytes before continuing; a publisher SHA-256 can allow faster resume")
    } else if job.plan.is_some() {
        tr("Supported")
    } else if job.downloaded > 0 {
        tr("Starts from the beginning")
    } else {
        tr("Checked when downloading")
    }
}

pub(super) fn progress(job: &Job) -> Option<f32> {
    if job.status == Status::Complete {
        return Some(1.0);
    }
    if let Some(hls) = &job.hls {
        return Some(hls.completed.len() as f32 / hls.playlist.part_count().max(1) as f32);
    }
    job.total
        .filter(|total| *total > 0)
        .map(|total| (job.downloaded as f64 / total as f64).clamp(0.0, 1.0) as f32)
}

pub(super) fn eta(job: &Job, rate: f64) -> String {
    if job.status != Status::Downloading || !rate.is_finite() || rate <= 0.0 {
        return "—".into();
    }
    let Some(total) = job.total.filter(|total| *total > job.downloaded) else {
        return "—".into();
    };
    let seconds = ((total - job.downloaded) as f64 / rate).ceil() as u64;
    if seconds < 60 {
        tf!("{seconds} sec", seconds = seconds)
    } else if seconds < 3600 {
        tf!("{} min {} sec", seconds / 60, seconds % 60)
    } else if seconds < 86400 {
        tf!("{} hr {} min", seconds / 3600, seconds % 3600 / 60)
    } else {
        tf!("{} days", seconds / 86400)
    }
}

pub(super) fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < 4 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
