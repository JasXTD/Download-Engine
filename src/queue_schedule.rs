//! Recurring windows use the current local time zone and weekday of the start.
use anyhow::{Context, Result};
use chrono::{Datelike, Local, NaiveDate, NaiveTime, TimeZone, Timelike};

pub const EVERY_DAY: u8 = 0b111_1111;
pub const DAY_NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecurringSchedule {
    /// Monday is bit 0; Sunday is bit 6.
    pub days: u8,
    pub start_minute: u16,
    pub stop_minute: u16,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub start: i64,
    pub stop: i64,
}

impl RecurringSchedule {
    pub fn new(days: u8, start: &str, stop: &str) -> Result<Self> {
        let schedule = Self {
            days,
            start_minute: parse_clock(start).context("Invalid start time")?,
            stop_minute: parse_clock(stop).context("Invalid pause time")?,
            enabled: true,
        };
        schedule.validate()?;
        Ok(schedule)
    }

    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.days > 0 && self.days <= EVERY_DAY,
            "Choose at least one start day"
        );
        anyhow::ensure!(
            self.start_minute < 1440 && self.stop_minute < 1440,
            "Use a time from 00:00 to 23:59"
        );
        anyhow::ensure!(
            self.start_minute != self.stop_minute,
            "Start and pause times must be different"
        );
        Ok(())
    }

    pub fn overnight(&self) -> bool {
        self.stop_minute < self.start_minute
    }

    pub fn summary(&self) -> String {
        let days = if self.days == EVERY_DAY {
            "Every day".into()
        } else {
            DAY_NAMES
                .iter()
                .enumerate()
                .filter(|(i, _)| self.days & (1 << i) != 0)
                .map(|(_, name)| *name)
                .collect::<Vec<_>>()
                .join(", ")
        };
        format!(
            "{} · {}–{}{}",
            days,
            clock_label(self.start_minute),
            clock_label(self.stop_minute),
            if self.overnight() { " (next day)" } else { "" }
        )
    }

    pub fn active_window(&self, now: i64) -> Result<Option<Window>> {
        self.window_at(&Local, now)
    }

    /// The interval is start-inclusive and stop-exclusive. Check yesterday too,
    /// since an overnight window belongs to its start day, not its stop day.
    pub fn window_at<Tz: TimeZone>(&self, zone: &Tz, now: i64) -> Result<Option<Window>> {
        self.validate()?;
        let today = zone
            .timestamp_opt(now, 0)
            .single()
            .context("Invalid schedule clock")?
            .date_naive();
        for date in [today.pred_opt(), Some(today)].into_iter().flatten() {
            if let Some(window) = self.occurrence(zone, date)
                && window.start <= now
                && now < window.stop
            {
                return Ok(Some(window));
            }
        }
        Ok(None)
    }

    pub fn next_window<Tz: TimeZone>(&self, zone: &Tz, now: i64) -> Result<Option<Window>> {
        self.validate()?;
        let mut date = zone
            .timestamp_opt(now, 0)
            .single()
            .context("Invalid schedule clock")?
            .date_naive();
        // Two weeks cover a weekly occurrence skipped during a daylight-saving gap.
        for _ in 0..15 {
            if let Some(window) = self.occurrence(zone, date)
                && window.start > now
            {
                return Ok(Some(window));
            }
            let Some(next) = date.succ_opt() else {
                break;
            };
            date = next;
        }
        Ok(None)
    }

    fn occurrence<Tz: TimeZone>(&self, zone: &Tz, date: NaiveDate) -> Option<Window> {
        if self.days & (1 << date.weekday().num_days_from_monday()) == 0 {
            return None;
        }
        let stop_date = if self.overnight() {
            date.succ_opt()?
        } else {
            date
        };
        let start = date.and_hms_opt(
            u32::from(self.start_minute / 60),
            u32::from(self.start_minute % 60),
            0,
        )?;
        let stop = stop_date.and_hms_opt(
            u32::from(self.stop_minute / 60),
            u32::from(self.stop_minute % 60),
            0,
        )?;
        // A missing boundary skips the entire occurrence. On repeated clock times,
        // use the first instant for both boundaries; do not start the window twice.
        let start = zone.from_local_datetime(&start).earliest()?.timestamp();
        let stop = zone.from_local_datetime(&stop).earliest()?.timestamp();
        (stop > start).then_some(Window { start, stop })
    }
}

pub fn clock_label(minute: u16) -> String {
    format!("{:02}:{:02}", minute / 60, minute % 60)
}

fn parse_clock(value: &str) -> Result<u16> {
    let time =
        NaiveTime::parse_from_str(value.trim(), "%H:%M").context("Use HH:MM (24-hour clock)")?;
    Ok((time.hour() * 60 + time.minute()) as u16)
}
