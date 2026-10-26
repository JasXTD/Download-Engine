//! Shared payload budgets. A file's ranges and link checks use the same bucket;
//! all files share the global bucket. Limits live outside worker-owned Job JSON.
use crate::store::Store;
use anyhow::{Result, ensure};
use rusqlite::{Connection, params};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::Notify, time::Instant};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub const MIN_RATE: u64 = 1024;
pub const MAX_RATE: u64 = 1024 * 1024 * 1024;
const MAX_GRANT: usize = 64 * 1024;
const MAX_CREDIT: u64 = 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Limits {
    pub global: Option<u64>,
    pub downloads: HashMap<String, u64>,
}
#[derive(Debug)]
pub struct Update {
    pub request: Uuid,
    pub global: Option<u64>,
    pub download: Option<(String, Option<u64>)>,
}
#[derive(Debug)]
pub struct Event {
    pub request: Uuid,
    pub result: std::result::Result<Limits, String>,
}
pub fn validate(rate: Option<u64>) -> Result<()> {
    ensure!(
        rate.is_none_or(|n| (MIN_RATE..=MAX_RATE).contains(&n)),
        "Enter a speed from 1 KiB/s to 1024 MiB/s, or turn off the limit."
    );
    Ok(())
}
pub(crate) fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS bandwidth_settings (
        id INTEGER PRIMARY KEY CHECK(id=1), bytes_per_second INTEGER
        CHECK(bytes_per_second BETWEEN 1024 AND 1073741824));
        INSERT OR IGNORE INTO bandwidth_settings(id,bytes_per_second) VALUES(1,NULL);
        CREATE TABLE IF NOT EXISTS bandwidth_downloads (
        job_id TEXT PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
        bytes_per_second INTEGER NOT NULL CHECK(bytes_per_second BETWEEN 1024 AND 1073741824));",
    )?;
    Ok(())
}
fn read(conn: &Connection) -> Result<Limits> {
    let global = conn.query_row(
        "SELECT bytes_per_second FROM bandwidth_settings WHERE id=1",
        [],
        |r| r.get::<_, Option<u32>>(0),
    )?;
    let global = global.map(u64::from);
    validate(global)?;
    let mut statement = conn.prepare("SELECT job_id,bytes_per_second FROM bandwidth_downloads")?;
    let downloads: HashMap<String, u64> = statement
        .query_map([], |r| Ok((r.get(0)?, u64::from(r.get::<_, u32>(1)?))))?
        .collect::<std::result::Result<_, _>>()?;
    for rate in downloads.values() {
        validate(Some(*rate))?;
    }
    Ok(Limits { global, downloads })
}
impl Store {
    pub fn bandwidth_limits(&self) -> Result<Limits> {
        let mut conn = self.connect()?;
        let tx = conn.transaction()?;
        read(&tx)
    }
    pub fn set_bandwidth_limits(&self, update: &Update) -> Result<Limits> {
        validate(update.global)?;
        if let Some((_, rate)) = &update.download {
            validate(*rate)?;
        }
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some((job, rate)) = &update.download {
            ensure!(
                tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM jobs WHERE id=?1)",
                    [job],
                    |r| r.get::<_, bool>(0)
                )?,
                "This download was removed. Close Speed limits and open it again."
            );
            if let Some(rate) = rate {
                tx.execute(
                    "INSERT INTO bandwidth_downloads(job_id,bytes_per_second) VALUES(?1,?2)
                    ON CONFLICT(job_id) DO UPDATE SET bytes_per_second=excluded.bytes_per_second",
                    params![job, *rate as u32],
                )?;
            } else {
                tx.execute("DELETE FROM bandwidth_downloads WHERE job_id=?1", [job])?;
            }
        }
        tx.execute(
            "UPDATE bandwidth_settings SET bytes_per_second=?1 WHERE id=1",
            [update.global.map(|n| n as u32)],
        )?;
        let limits = read(&tx)?;
        tx.commit()?;
        Ok(limits)
    }
}

struct Bucket {
    rate: u64,
    tokens: f64,
    updated: Instant,
}
impl Bucket {
    fn new(rate: u64, now: Instant) -> Self {
        Self {
            rate,
            tokens: 0.0,
            updated: now,
        }
    }
    fn quantum(&self) -> usize {
        ((self.rate / 20) as usize).clamp(1, MAX_GRANT)
    }
    fn refill(&mut self, now: Instant) {
        self.tokens = (self.tokens
            + now.duration_since(self.updated).as_secs_f64() * self.rate as f64)
            // Preserve late timer credit instead of losing it every time a
            // Windows timer wakes after its deadline. Bound idle-time bursts.
            .min((self.rate / 10).min(MAX_CREDIT) as f64);
        self.updated = now;
    }
    fn wait(&self, n: usize) -> Duration {
        Duration::from_secs_f64(((n as f64 - self.tokens) / self.rate as f64).max(0.0))
    }
}
struct Waiter {
    id: u64,
    job: String,
    max: usize,
    ready: Arc<Notify>,
}
#[derive(Default)]
struct State {
    global: Option<Bucket>,
    downloads: HashMap<String, Bucket>,
    waiters: VecDeque<Waiter>,
    next: u64,
}
impl State {
    fn ready(&mut self) -> (Option<(u64, usize)>, Option<Duration>) {
        let now = Instant::now();
        if let Some(bucket) = &mut self.global {
            bucket.refill(now);
        }
        for waiter in &self.waiters {
            if let Some(bucket) = self.downloads.get_mut(&waiter.job) {
                bucket.refill(now);
            }
        }
        let mut delay: Option<Duration> = None;
        let mut eligible = false;
        for waiter in &self.waiters {
            let file = self.downloads.get(&waiter.job);
            let n = waiter
                .max
                .min(MAX_GRANT)
                .min(self.global.as_ref().map_or(MAX_GRANT, Bucket::quantum))
                .min(file.map_or(MAX_GRANT, Bucket::quantum));
            let file_wait = file.map_or(Duration::ZERO, |b| b.wait(n));
            if !file_wait.is_zero() {
                delay = Some(delay.map_or(file_wait, |d| d.min(file_wait)));
            } else if !eligible {
                // Oldest file with credit gets first access to global credit.
                // A file waiting on its own cap cannot hold up other files.
                eligible = true;
                let global_wait = self.global.as_ref().map_or(Duration::ZERO, |b| b.wait(n));
                if global_wait.is_zero() {
                    return (Some((waiter.id, n)), None);
                }
                delay = Some(delay.map_or(global_wait, |d| d.min(global_wait)));
            }
        }
        (None, delay)
    }
}
#[derive(Clone, Default)]
pub struct Limiter {
    state: Arc<Mutex<State>>,
    changed: Arc<Notify>,
    quota: Option<crate::quota::Meter>,
}
impl Limiter {
    pub(crate) fn with_store(&self, store: &Store) -> Result<Self> {
        let mut limiter = self.clone();
        if limiter.quota.is_none() {
            limiter.quota = Some(crate::quota::Meter::new(store.clone())?);
        }
        Ok(limiter)
    }
    pub(crate) async fn check_quota(&self) -> Result<()> {
        if let Some(meter) = &self.quota {
            meter.check().await?;
        }
        Ok(())
    }
    pub(crate) async fn record_payload(&self, bytes: usize) -> Result<()> {
        if let Some(meter) = &self.quota {
            meter.record(bytes).await?;
        }
        Ok(())
    }
    pub(crate) async fn quota_status(&self) -> Result<crate::quota::Status> {
        self.quota
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Data quota accounting is unavailable"))?
            .status()
            .await
    }
    pub(crate) async fn set_quota_limits(
        &self,
        limits: crate::quota::Limits,
    ) -> Result<crate::quota::Status> {
        self.quota
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Data quota accounting is unavailable"))?
            .set_limits(limits)
            .await
    }
    pub fn new(limits: &Limits) -> Result<Self> {
        let result = Self::default();
        result.update(limits)?;
        Ok(result)
    }
    pub fn update(&self, limits: &Limits) -> Result<()> {
        validate(limits.global)?;
        for rate in limits.downloads.values() {
            validate(Some(*rate))?;
        }
        let mut state = self.state.lock().unwrap();
        let now = Instant::now();
        if state.global.as_ref().map(|b| b.rate) != limits.global {
            state.global = limits.global.map(|rate| Bucket::new(rate, now));
        }
        state
            .downloads
            .retain(|job, _| limits.downloads.contains_key(job));
        for (job, rate) in &limits.downloads {
            if state.downloads.get(job).map(|b| b.rate) != Some(*rate) {
                state.downloads.insert(job.clone(), Bucket::new(*rate, now));
            }
        }
        drop(state);
        self.changed.notify_waiters();
        Ok(())
    }
    pub(crate) fn forget(&self, job: &str) {
        self.state.lock().unwrap().downloads.remove(job);
        self.changed.notify_waiters();
    }
    /// Charge both budgets atomically. None means cancellation. Dropping this
    /// future also removes its ticket, so a cancelled link check cannot block transfers.
    pub async fn acquire(
        &self,
        job: &str,
        max: usize,
        cancel: &CancellationToken,
    ) -> Option<usize> {
        assert!(max > 0);
        if cancel.is_cancelled() {
            return None;
        }
        let (id, ready_signal) = {
            let mut state = self.state.lock().unwrap();
            if state.global.is_none() && !state.downloads.contains_key(job) {
                return Some(max);
            }
            let ready_signal = Arc::new(Notify::new());
            let id = state.next;
            state.next += 1;
            state.waiters.push_back(Waiter {
                id,
                job: job.into(),
                max,
                ready: ready_signal.clone(),
            });
            (id, ready_signal)
        };
        let _ticket = Ticket { limiter: self, id };
        self.changed.notify_waiters();
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable(); // Register before inspecting state: no lost wakeups.
            let ready_notification = ready_signal.notified();
            tokio::pin!(ready_notification);
            ready_notification.as_mut().enable();
            let delay = {
                let mut state = self.state.lock().unwrap();
                let (ready, delay) = state.ready();
                if let Some((next, n)) = ready {
                    if next == id {
                        if let Some(b) = &mut state.global {
                            b.tokens -= n as f64;
                        }
                        if let Some(b) = state.downloads.get_mut(job) {
                            b.tokens -= n as f64;
                        }
                        state.waiters.retain(|w| w.id != id);
                        return Some(n);
                    }
                    // Eligibility can change while two workers are waking: the
                    // new owner may have dropped its timer while waiting for us.
                    // Wake only that owner; a global broadcast would also wake
                    // this waiter repeatedly without making progress.
                    if let Some(owner) = state.waiters.iter().find(|w| w.id == next) {
                        owner.ready.notify_one();
                    }
                    None
                } else {
                    delay
                }
            };
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return None,
                _ = &mut ready_notification => {},
                _ = &mut notified => {},
                _ = async { match delay {
                    Some(d) => tokio::time::sleep(d.max(Duration::from_millis(1))).await,
                    None => std::future::pending::<()>().await,
                }} => {},
            }
        }
    }
}
struct Ticket<'a> {
    limiter: &'a Limiter,
    id: u64,
}
impl Drop for Ticket<'_> {
    fn drop(&mut self) {
        self.limiter
            .state
            .lock()
            .unwrap()
            .waiters
            .retain(|w| w.id != self.id);
        self.limiter.changed.notify_waiters();
    }
}
