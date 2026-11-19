//! Recurring windows use the current local time zone and weekday of the start.
use anyhow::{Context, Result};
use jiff::{
    Timestamp,
    civil::{Date, Time},
    tz::{AmbiguousOffset, TimeZone},
};

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
        self.window_at(
            &TimeZone::try_system().context("Cannot determine the local time zone")?,
            now,
        )
    }

    /// The interval is start-inclusive and stop-exclusive. Check yesterday too,
    /// since an overnight window belongs to its start day, not its stop day.
    pub fn window_at(&self, zone: &TimeZone, now: i64) -> Result<Option<Window>> {
        self.validate()?;
        let today = zone
            .to_datetime(Timestamp::from_second(now).context("Invalid schedule clock")?)
            .date();
        for date in [today.yesterday().ok(), Some(today)].into_iter().flatten() {
            if let Some(window) = self.occurrence(zone, date)
                && window.start <= now
                && now < window.stop
            {
                return Ok(Some(window));
            }
        }
        Ok(None)
    }

    pub fn next_window(&self, zone: &TimeZone, now: i64) -> Result<Option<Window>> {
        self.validate()?;
        let mut date = zone
            .to_datetime(Timestamp::from_second(now).context("Invalid schedule clock")?)
            .date();
        // Two weeks cover a weekly occurrence skipped during a daylight-saving gap.
        for _ in 0..15 {
            if let Some(window) = self.occurrence(zone, date)
                && window.start > now
            {
                return Ok(Some(window));
            }
            let Ok(next) = date.tomorrow() else {
                break;
            };
            date = next;
        }
        Ok(None)
    }

    fn occurrence(&self, zone: &TimeZone, date: Date) -> Option<Window> {
        if self.days & (1 << date.weekday().to_monday_zero_offset()) == 0 {
            return None;
        }
        let stop_date = if self.overnight() {
            date.tomorrow().ok()?
        } else {
            date
        };
        let start = date.at(
            (self.start_minute / 60) as i8,
            (self.start_minute % 60) as i8,
            0,
            0,
        );
        let stop = stop_date.at(
            (self.stop_minute / 60) as i8,
            (self.stop_minute % 60) as i8,
            0,
            0,
        );
        // A missing boundary skips the entire occurrence. On repeated clock times,
        // use the first instant for both boundaries; do not start the window twice.
        let boundary = |value| {
            let ambiguous = zone.to_ambiguous_timestamp(value);
            // Jiff's `earlier` also resolves gaps, which must remain skipped here.
            if matches!(ambiguous.offset(), AmbiguousOffset::Gap { .. }) {
                return None;
            }
            ambiguous.earlier().ok().map(|instant| instant.as_second())
        };
        let start = boundary(start)?;
        let stop = boundary(stop)?;
        (stop > start).then_some(Window { start, stop })
    }
}

pub fn clock_label(minute: u16) -> String {
    format!("{:02}:{:02}", minute / 60, minute % 60)
}

fn parse_clock(value: &str) -> Result<u16> {
    let time = Time::strptime("%H:%M", value.trim()).context("Use HH:MM (24-hour clock)")?;
    Ok(time.hour() as u16 * 60 + time.minute() as u16)
}
