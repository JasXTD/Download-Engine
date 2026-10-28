#[path = "../src/desktop/fixture_database.rs"]
mod fixture_database;
use chrono::{FixedOffset, Local, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};
use download_engine::{
    paths::AppPaths,
    queue::MAIN_QUEUE,
    queue_schedule::{EVERY_DAY, RecurringSchedule},
    store::Store,
};

fn utc(value: &str) -> i64 {
    parse(value).and_utc().timestamp()
}
fn parse(value: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M").unwrap()
}
fn local(value: &str) -> i64 {
    Local
        .from_local_datetime(&parse(value))
        .single()
        .unwrap()
        .timestamp()
}
fn setup() -> (tempfile::TempDir, Store) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("files"),
    })
    .unwrap();
    (temp, store)
}

#[test]
fn recurring_input_rejects_invalid_days_times_and_zero_length_windows() {
    for (days, start, stop) in [
        (0, "01:00", "07:00"),
        (128, "01:00", "07:00"),
        (EVERY_DAY, "24:00", "07:00"),
        (EVERY_DAY, "01:00", "07:60"),
        (EVERY_DAY, "01:00", "01:00"),
        (EVERY_DAY, "tomorrow", "07:00"),
    ] {
        assert!(RecurringSchedule::new(days, start, stop).is_err());
    }
    let window = RecurringSchedule::new(EVERY_DAY, "23:00", "00:00").unwrap();
    assert!(window.overnight());
    assert_eq!(window.summary(), "Every day · 23:00–00:00 (next day)");
    let invalid = RecurringSchedule {
        days: 1,
        start_minute: 1440,
        stop_minute: 100,
        enabled: true,
    };
    assert!(invalid.window_at(&Utc, utc("2026-10-12 00:00")).is_err());
}

#[test]
fn daily_windows_include_start_exclude_stop_and_resume_on_the_next_day() {
    let schedule = RecurringSchedule::new(EVERY_DAY, "00:00", "07:00").unwrap();
    for (date, active) in [
        ("2026-10-11 23:59", false),
        ("2026-10-12 00:00", true),
        ("2026-10-12 06:59", true),
        ("2026-10-12 07:00", false),
        ("2026-10-13 00:00", true),
    ] {
        assert_eq!(
            schedule.window_at(&Utc, utc(date)).unwrap().is_some(),
            active,
            "{date}"
        );
    }
    assert_eq!(
        schedule
            .next_window(&Utc, utc("2026-10-12 07:00"))
            .unwrap()
            .unwrap()
            .start,
        utc("2026-10-13 00:00")
    );
}

#[test]
fn overnight_windows_follow_start_weekdays_across_week_and_year_boundaries() {
    let sunday = RecurringSchedule::new(1 << 6, "23:00", "07:00").unwrap();
    for (date, active) in [
        ("2026-10-11 22:59", false),
        ("2026-10-11 23:00", true),
        ("2026-10-12 06:59", true),
        ("2026-10-12 07:00", false),
        ("2026-10-12 23:00", false),
    ] {
        assert_eq!(
            sunday.window_at(&Utc, utc(date)).unwrap().is_some(),
            active,
            "{date}"
        );
    }
    assert_eq!(
        sunday
            .next_window(&Utc, utc("2026-10-12 06:00"))
            .unwrap()
            .unwrap()
            .start,
        utc("2026-10-18 23:00")
    );
    let daily = RecurringSchedule::new(EVERY_DAY, "23:00", "07:00").unwrap();
    assert_eq!(
        daily
            .window_at(&Utc, utc("2027-01-01 01:00"))
            .unwrap()
            .unwrap()
            .start,
        utc("2026-12-31 23:00")
    );
    let weekdays = RecurringSchedule::new(0b001_1111, "09:00", "17:00").unwrap();
    assert!(
        weekdays
            .window_at(&Utc, utc("2026-10-17 10:00"))
            .unwrap()
            .is_none()
    );
    assert!(
        weekdays
            .window_at(&Utc, utc("2026-10-19 10:00"))
            .unwrap()
            .is_some()
    );
}

#[test]
fn schedule_follows_the_current_zone_and_clock_instead_of_replaying_missed_windows() {
    let schedule = RecurringSchedule::new(EVERY_DAY, "00:00", "07:00").unwrap();
    let at = utc("2026-10-12 01:00");
    assert!(schedule.window_at(&Utc, at).unwrap().is_some());
    assert!(
        schedule
            .window_at(&FixedOffset::east_opt(8 * 3600).unwrap(), at)
            .unwrap()
            .is_none()
    );
    assert!(
        schedule
            .window_at(&Utc, utc("2026-11-16 12:00"))
            .unwrap()
            .is_none()
    );
    // Moving the clock back into a window follows its current eligibility.
    assert!(schedule.window_at(&Utc, at - 60).unwrap().is_some());
}

// A deterministic zone fixture with the actual 2026 US Eastern DST transitions.
// Resolve local time by round-tripping both offsets through the UTC rule.
#[derive(Clone)]
struct Eastern2026;
impl TimeZone for Eastern2026 {
    type Offset = FixedOffset;
    fn from_offset(_: &FixedOffset) -> Self {
        Self
    }
    fn offset_from_local_date(&self, date: &NaiveDate) -> LocalResult<FixedOffset> {
        self.offset_from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
    }
    fn offset_from_local_datetime(&self, value: &NaiveDateTime) -> LocalResult<FixedOffset> {
        let offsets: Vec<_> = [4, 5]
            .into_iter()
            .map(|hours| FixedOffset::west_opt(hours * 3600).unwrap())
            .filter(|offset| self.offset_from_utc_datetime(&(*value - *offset)) == *offset)
            .collect();
        match offsets.as_slice() {
            [] => LocalResult::None,
            [only] => LocalResult::Single(*only),
            [first, second] => LocalResult::Ambiguous(*first, *second),
            _ => unreachable!(),
        }
    }
    fn offset_from_utc_date(&self, date: &NaiveDate) -> FixedOffset {
        self.offset_from_utc_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
    }
    fn offset_from_utc_datetime(&self, value: &NaiveDateTime) -> FixedOffset {
        let daylight = *value >= parse("2026-03-08 07:00") && *value < parse("2026-11-01 06:00");
        FixedOffset::west_opt(if daylight { 4 * 3600 } else { 5 * 3600 }).unwrap()
    }
}

#[test]
fn daylight_saving_gaps_skip_the_whole_window_and_repeats_do_not_restart_it() {
    let zone = Eastern2026;
    let missing_start = RecurringSchedule::new(1 << 6, "02:30", "04:00").unwrap();
    assert!(
        missing_start
            .window_at(&zone, utc("2026-03-08 07:45"))
            .unwrap()
            .is_none()
    );
    assert_eq!(
        missing_start
            .next_window(&zone, utc("2026-03-08 06:00"))
            .unwrap()
            .unwrap()
            .start,
        utc("2026-03-15 06:30")
    );
    let missing_stop = RecurringSchedule::new(1 << 5, "23:00", "02:30").unwrap();
    assert!(
        missing_stop
            .window_at(&zone, utc("2026-03-08 04:30"))
            .unwrap()
            .is_none()
    );
    let repeated = RecurringSchedule::new(EVERY_DAY, "01:15", "01:45").unwrap();
    assert!(
        repeated
            .window_at(&zone, utc("2026-11-01 05:30"))
            .unwrap()
            .is_some()
    );
    assert!(
        repeated
            .window_at(&zone, utc("2026-11-01 05:45"))
            .unwrap()
            .is_none()
    );
    assert!(
        repeated
            .window_at(&zone, utc("2026-11-01 06:30"))
            .unwrap()
            .is_none()
    );
    let spanning = RecurringSchedule::new(EVERY_DAY, "00:00", "03:00").unwrap();
    let spring = spanning
        .window_at(&zone, utc("2026-03-08 06:00"))
        .unwrap()
        .unwrap();
    let fall = spanning
        .window_at(&zone, utc("2026-11-01 06:00"))
        .unwrap()
        .unwrap();
    assert_eq!(spring.stop - spring.start, 2 * 3600);
    assert_eq!(fall.stop - fall.start, 4 * 3600);
}

#[test]
fn stored_recurrence_survives_restart_and_manual_controls_suspend_it() {
    let (_temp, store) = setup();
    let schedule = RecurringSchedule::new(1, "23:00", "07:00").unwrap();
    let before = local("2026-10-12 22:00");
    let inside = local("2026-10-13 01:00");
    let after = local("2026-10-13 08:00");
    store
        .schedule_recurring_queue(MAIN_QUEUE, &schedule, before)
        .unwrap();
    let store = Store::open(store.paths.clone()).unwrap();
    assert_eq!(store.queues().unwrap()[0].recurring, Some(schedule.clone()));
    assert!(!store.queues().unwrap()[0].running);
    assert_eq!(store.apply_queue_schedules(inside).unwrap(), 1);
    assert_eq!(store.apply_queue_schedules(inside).unwrap(), 0);
    assert!(store.queues().unwrap()[0].running);
    assert_eq!(store.apply_queue_schedules(after).unwrap(), 1);
    // A manual start outside the window remains in effect on subsequent ticks.
    store.set_queue_running(MAIN_QUEUE, true).unwrap();
    assert_eq!(store.apply_queue_schedules(after).unwrap(), 0);
    assert!(store.queues().unwrap()[0].running);
    assert!(
        !store.queues().unwrap()[0]
            .recurring
            .as_ref()
            .unwrap()
            .enabled
    );
    store.enable_recurring_schedule(MAIN_QUEUE, after).unwrap();
    assert!(!store.queues().unwrap()[0].running);
    store.enable_recurring_schedule(MAIN_QUEUE, inside).unwrap();
    assert!(store.queues().unwrap()[0].running);
    store.set_queue_running(MAIN_QUEUE, false).unwrap();
    let reopened = Store::open(store.paths.clone()).unwrap();
    assert_eq!(reopened.apply_queue_schedules(inside).unwrap(), 0);
    assert_eq!(
        reopened
            .apply_queue_schedules(local("2026-10-19 23:00"))
            .unwrap(),
        0
    );
    assert!(!reopened.queues().unwrap()[0].running);
    assert!(
        reopened.queues().unwrap()[0]
            .state_label(inside)
            .contains("suspended")
    );
}

#[test]
fn missed_windows_do_not_run_on_reopen_and_saved_schedule_can_be_replaced_or_cleared() {
    let (_temp, store) = setup();
    let at = local("2026-10-12 01:00");
    let rule = RecurringSchedule::new(EVERY_DAY, "00:00", "07:00").unwrap();
    store
        .schedule_recurring_queue(MAIN_QUEUE, &rule, at)
        .unwrap();
    assert!(store.queues().unwrap()[0].running);
    let reopened = Store::open(store.paths.clone()).unwrap();
    reopened
        .apply_queue_schedules(local("2026-10-15 12:00"))
        .unwrap();
    assert!(!reopened.queues().unwrap()[0].running);
    reopened
        .apply_queue_schedules(local("2026-10-16 01:00"))
        .unwrap();
    assert!(reopened.queues().unwrap()[0].running);
    reopened.clear_schedule(MAIN_QUEUE).unwrap();
    reopened
        .apply_queue_schedules(local("2026-10-16 12:00"))
        .unwrap();
    assert!(reopened.queues().unwrap()[0].running);
    assert!(!reopened.queues().unwrap()[0].has_schedule());
    assert!(reopened.enable_recurring_schedule(MAIN_QUEUE, at).is_err());
    reopened
        .schedule_recurring_queue(MAIN_QUEUE, &rule, at)
        .unwrap();
    reopened.schedule_queue(MAIN_QUEUE, at + 60, at).unwrap();
    assert!(reopened.queues().unwrap()[0].recurring.is_none());
    reopened
        .schedule_recurring_queue(MAIN_QUEUE, &rule, at)
        .unwrap();
    assert!(reopened.queues().unwrap()[0].scheduled_at.is_none());
}

#[test]
fn invalid_schedule_updates_leave_existing_rules_and_queue_state_intact() {
    let (_temp, store) = setup();
    let at = local("2026-10-12 01:00");
    let rule = RecurringSchedule::new(EVERY_DAY, "00:00", "07:00").unwrap();
    store
        .schedule_recurring_queue(MAIN_QUEUE, &rule, at)
        .unwrap();
    let saved = store.queues().unwrap();
    let mut invalid = rule.clone();
    invalid.days = 0;
    assert!(
        store
            .schedule_recurring_queue(MAIN_QUEUE, &invalid, at)
            .is_err()
    );
    assert!(
        store
            .schedule_recurring_queue("missing", &rule, at)
            .is_err()
    );
    assert!(store.schedule_queue(MAIN_QUEUE, at - 1, at).is_err());
    assert_eq!(store.queues().unwrap(), saved);
    let conn = crate::fixture_database::open(store.paths.data.join("queue.sqlite3")).unwrap();
    assert!(
        conn.execute("UPDATE queue_schedules SET days=128", [])
            .is_err()
    );
    assert!(
        conn.execute("UPDATE queue_schedules SET stop_minute=start_minute", [])
            .is_err()
    );
    assert_eq!(store.queues().unwrap(), saved);
    let queue = store.create_queue("Disposable").unwrap();
    store
        .schedule_recurring_queue(&queue.id, &rule, at)
        .unwrap();
    store.delete_queue(&queue.id).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM queue_schedules WHERE queue_id=?1",
            [&queue.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}
