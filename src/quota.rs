//! Persistent payload accounting. Limits never change queue or manual-pause ownership.
use crate::store::Store;
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, TransactionBehavior, params};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use uuid::Uuid;

pub const MIN_LIMIT: u64 = 1024 * 1024;
pub const MAX_LIMIT: u64 = 1024 * 1024 * 1024 * 1024 * 1024;
pub const HOUR: i64 = 3600;
pub const DAY: i64 = 24 * HOUR;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Limits {
    pub hourly: Option<u64>,
    pub daily: Option<u64>,
}
impl Limits {
    pub fn enabled(self) -> bool {
        self.hourly.is_some() || self.daily.is_some()
    }
    pub fn validate(self) -> Result<()> {
        for limit in [self.hourly, self.daily] {
            ensure!(
                limit.is_none_or(|n| (MIN_LIMIT..=MAX_LIMIT).contains(&n)),
                "Choose a data limit from 1 MiB to 1 PiB, or turn off the limit."
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub limits: Limits,
    pub hourly_used: u64,
    pub daily_used: u64,
    pub hour_start: Option<i64>,
    pub day_start: Option<i64>,
}
impl Status {
    pub fn blocked(&self) -> bool {
        self.limits.hourly.is_some_and(|n| self.hourly_used >= n)
            || self.limits.daily.is_some_and(|n| self.daily_used >= n)
    }
    pub fn resumes_at(&self) -> Option<i64> {
        let hour = self
            .limits
            .hourly
            .filter(|n| self.hourly_used >= *n)
            .and(self.hour_start)
            .map(|at| at + HOUR);
        let day = self
            .limits
            .daily
            .filter(|n| self.daily_used >= *n)
            .and(self.day_start)
            .map(|at| at + DAY);
        hour.into_iter().chain(day).max()
    }
    fn advance(&mut self, now: i64) -> Result<()> {
        ensure!(
            (0..=i64::MAX - DAY).contains(&now),
            "Invalid data quota time"
        );
        for (start, used, period) in [
            (&mut self.hour_start, &mut self.hourly_used, HOUR),
            (&mut self.day_start, &mut self.daily_used, DAY),
        ] {
            if let Some(at) = *start {
                if now >= at + period {
                    *start = Some(now - (now - at) % period);
                    *used = 0;
                }
                // A clock rollback never grants a second allowance.
            } else if self.limits.enabled() {
                *start = Some(now);
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct Update {
    pub request: Uuid,
    pub limits: Limits,
}
#[derive(Debug)]
pub struct Event {
    pub request: Uuid,
    pub result: std::result::Result<Status, String>,
}

#[derive(Debug)]
pub(crate) struct Reached;
impl std::fmt::Display for Reached {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Data quota reached. Downloads will continue when the allowance resets.")
    }
}
impl std::error::Error for Reached {}
pub(crate) fn reached(error: &anyhow::Error) -> bool {
    error.downcast_ref::<Reached>().is_some()
}

pub(crate) fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS data_quotas (
         id INTEGER PRIMARY KEY CHECK(id=1),
         hourly_limit INTEGER CHECK(hourly_limit BETWEEN 1048576 AND 1125899906842624),
         daily_limit INTEGER CHECK(daily_limit BETWEEN 1048576 AND 1125899906842624),
         hourly_used INTEGER NOT NULL CHECK(hourly_used>=0),
         daily_used INTEGER NOT NULL CHECK(daily_used>=0),
         hour_start INTEGER CHECK(hour_start>=0), day_start INTEGER CHECK(day_start>=0));
         INSERT OR IGNORE INTO data_quotas VALUES(1,NULL,NULL,0,0,NULL,NULL);",
    )?;
    Ok(())
}
fn read(conn: &Connection) -> Result<Status> {
    let status = conn.query_row(
        "SELECT hourly_limit,daily_limit,hourly_used,daily_used,hour_start,day_start FROM data_quotas WHERE id=1", [],
        |r| Ok(Status { limits: Limits {
            hourly: r.get::<_, Option<i64>>(0)?.map(|n| n as u64),
            daily: r.get::<_, Option<i64>>(1)?.map(|n| n as u64) },
            hourly_used: read_usage(r, 2)?, daily_used: read_usage(r, 3)?,
            hour_start: r.get(4)?, day_start: r.get(5)? }),
    )?;
    status.limits.validate()?;
    ensure!(
        [status.hour_start, status.day_start]
            .into_iter()
            .flatten()
            .all(|at| (0..=i64::MAX - DAY).contains(&at)),
        "Invalid saved quota window"
    );
    Ok(status)
}
fn read_usage(row: &rusqlite::Row<'_>, column: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(column)?;
    value
        .try_into()
        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(column, value))
}
fn write(conn: &Connection, status: &Status) -> Result<()> {
    let n = conn.prepare_cached(
        "UPDATE data_quotas SET hourly_limit=?1,daily_limit=?2,hourly_used=?3,daily_used=?4,hour_start=?5,day_start=?6 WHERE id=1")?.execute(
        params![status.limits.hourly.map(|n| n as i64), status.limits.daily.map(|n| n as i64),
            i64::try_from(status.hourly_used)?, i64::try_from(status.daily_used)?, status.hour_start, status.day_start],
    )?;
    ensure!(n == 1, "Data quota settings are unavailable");
    Ok(())
}
impl Store {
    pub fn quota_status(&self) -> Result<Status> {
        self.quota_status_at(chrono::Utc::now().timestamp())
    }
    pub fn quota_status_at(&self, now: i64) -> Result<Status> {
        self.with_connection(|conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut status = read(&tx)?;
            let previous = status.clone();
            status.advance(now)?;
            if status != previous {
                write(&tx, &status)?;
            }
            tx.commit()?;
            Ok(status)
        })
    }
    pub fn set_quota_limits_at(&self, limits: Limits, now: i64) -> Result<Status> {
        limits.validate()?;
        self.with_connection(|conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut status = read(&tx)?;
            status.limits = limits;
            status.advance(now)?;
            write(&tx, &status)?;
            tx.commit()?;
            Ok(status)
        })
    }
    /// Account received payload before processing it, including bytes subsequently
    /// discarded after cancellation, a failed write, or a retry. Never refund data.
    pub fn record_quota_bytes_at(&self, bytes: u64, now: i64) -> Result<Status> {
        self.with_connection(|conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut status = read(&tx)?;
            let previous = status.clone();
            status.advance(now)?;
            if status.limits.enabled() {
                status.hourly_used = status
                    .hourly_used
                    .checked_add(bytes)
                    .filter(|n| *n <= i64::MAX as u64)
                    .context("Hourly usage overflow")?;
                status.daily_used = status
                    .daily_used
                    .checked_add(bytes)
                    .filter(|n| *n <= i64::MAX as u64)
                    .context("Daily usage overflow")?;
            }
            if status != previous {
                write(&tx, &status)?;
            }
            tx.commit()?;
            Ok(status)
        })
    }
}

struct Inner {
    store: Store,
    // Serialize refresh, writes, and settings changes so a late write cannot
    // restore an older cache after the UI has acknowledged a new limit.
    status: Mutex<Status>,
    enabled: AtomicBool,
    blocked: AtomicBool,
}
#[derive(Clone)]
pub(crate) struct Meter(Arc<Inner>);
impl Meter {
    pub(crate) fn new(store: Store) -> Result<Self> {
        let status = store.quota_status()?;
        Ok(Self(Arc::new(Inner {
            store,
            enabled: AtomicBool::new(status.limits.enabled()),
            blocked: AtomicBool::new(status.blocked()),
            status: Mutex::new(status),
        })))
    }
    async fn update(
        &self,
        operation: impl FnOnce(&Store) -> Result<Status> + Send + 'static,
    ) -> Result<Status> {
        let inner = self.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut cached = inner
                .status
                .lock()
                .map_err(|_| anyhow::anyhow!("Data quota accounting was interrupted"))?;
            let status = operation(&inner.store)?;
            inner
                .enabled
                .store(status.limits.enabled(), Ordering::Release);
            inner.blocked.store(status.blocked(), Ordering::Release);
            *cached = status.clone();
            Ok(status)
        })
        .await?
    }
    pub(crate) async fn status(&self) -> Result<Status> {
        self.update(Store::quota_status).await
    }
    pub(crate) async fn set_limits(&self, limits: Limits) -> Result<Status> {
        self.update(move |store| store.set_quota_limits_at(limits, chrono::Utc::now().timestamp()))
            .await
    }
    pub(crate) async fn check(&self) -> Result<()> {
        if self.0.blocked.load(Ordering::Acquire) && self.status().await?.blocked() {
            return Err(Reached.into());
        }
        Ok(())
    }
    pub(crate) async fn record(&self, bytes: usize) -> Result<()> {
        if self.0.enabled.load(Ordering::Acquire) && bytes != 0 {
            // Window selection follows the serialized accounting order, including
            // a chunk received just before a reset whose commit happens after it.
            self.update(move |store| {
                store.record_quota_bytes_at(bytes as u64, chrono::Utc::now().timestamp())
            })
            .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (tempfile::TempDir, Store) {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(crate::paths::AppPaths {
            data: temp.path().join("data"),
            downloads: temp.path().join("downloads"),
        })
        .unwrap();
        (temp, store)
    }
    #[tokio::test]
    async fn quota_shared_meter_applies_live_settings_and_refreshes_after_reset() {
        let (_temp, store) = setup();
        let meter = Meter::new(store.clone()).unwrap();
        meter.record(123).await.unwrap();
        assert_eq!(store.quota_status().unwrap().daily_used, 0);
        meter
            .set_limits(Limits {
                hourly: Some(MIN_LIMIT),
                daily: Some(2 * MIN_LIMIT),
            })
            .await
            .unwrap();
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..8 {
            let meter = meter.clone();
            tasks.spawn(async move {
                for _ in 0..8 {
                    meter.record(32768).await.unwrap();
                }
            });
        }
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
        assert_eq!(meter.status().await.unwrap().hourly_used, 2 * MIN_LIMIT);
        assert!(reached(&meter.check().await.unwrap_err()));
        meter
            .set_limits(Limits {
                hourly: Some(4 * MIN_LIMIT),
                daily: None,
            })
            .await
            .unwrap();
        meter.check().await.unwrap();
        meter
            .set_limits(Limits {
                hourly: Some(MIN_LIMIT),
                daily: None,
            })
            .await
            .unwrap();
        assert!(reached(&meter.check().await.unwrap_err()));
        // Move to the next accounting window without waiting an hour.
        store
            .quota_status_at(chrono::Utc::now().timestamp() + HOUR)
            .unwrap();
        meter.check().await.unwrap();
        assert_eq!(meter.status().await.unwrap().hourly_used, 0);
        assert_eq!(meter.status().await.unwrap().daily_used, 2 * MIN_LIMIT);
    }
    #[tokio::test]
    async fn quota_failed_accounting_and_updates_are_errors_without_optimistic_acknowledgement() {
        let (_temp, store) = setup();
        let meter = Meter::new(store.clone()).unwrap();
        meter
            .set_limits(Limits {
                hourly: Some(MIN_LIMIT),
                daily: None,
            })
            .await
            .unwrap();
        let conn = store.connect().unwrap();
        conn.execute_batch("CREATE TRIGGER reject_quota BEFORE UPDATE ON data_quotas BEGIN SELECT RAISE(ABORT,'fixture cannot persist usage'); END;").unwrap();
        assert!(
            meter
                .record(100)
                .await
                .unwrap_err()
                .to_string()
                .contains("fixture")
        );
        assert!(meter.set_limits(Limits::default()).await.is_err());
        assert!(store.quota_status().unwrap().limits.enabled());
        assert_eq!(store.quota_status().unwrap().hourly_used, 0);
        conn.execute_batch("DROP TRIGGER reject_quota;").unwrap();
        meter.record(100).await.unwrap();
        assert_eq!(meter.status().await.unwrap().hourly_used, 100);
    }
    #[test]
    fn quota_cannot_hide_validation_errors_or_consume_retry_budget() {
        let mut error = Some(anyhow::Error::from(Reached));
        crate::retry::merge_error(&mut error, crate::retry::incomplete("temporary"));
        assert!(reached(error.as_ref().unwrap()));
        crate::retry::merge_error(&mut error, anyhow::anyhow!("file changed"));
        assert_eq!(error.as_ref().unwrap().to_string(), "file changed");
        crate::retry::merge_error(&mut error, Reached.into());
        assert_eq!(error.as_ref().unwrap().to_string(), "file changed");
        let mut transient = Some(crate::retry::incomplete("temporary"));
        crate::retry::merge_error(&mut transient, Reached.into());
        assert!(reached(transient.as_ref().unwrap()));
    }
}
