//! Keeps a stream of bytes under a speed limit.

use std::sync::atomic::{AtomicU32, Ordering};

use parking_lot::Mutex;
use tokio::sync::Notify;
use tokio::time::{Duration, Instant};

/// A short burst is allowed, so that a small block doesn't wait for a whole time slice.
const MIN_BURST: f64 = 64.0 * 1024.0;

struct Balance {
    bytes: f64,
    updated: Instant,
}

pub struct ByteLimiter {
    /// Zero means no limit.
    bytes_per_second: AtomicU32,
    balance: Mutex<Balance>,
}

impl ByteLimiter {
    pub fn new(bytes_per_second: Option<u32>) -> Self {
        Self {
            bytes_per_second: AtomicU32::new(bytes_per_second.unwrap_or(0)),
            balance: Mutex::new(Balance {
                bytes: 0.0,
                updated: Instant::now(),
            }),
        }
    }

    /// Changes the limit. Anyone already waiting finishes the wait they were given.
    pub fn set_limit(&self, bytes_per_second: Option<u32>) {
        self.bytes_per_second
            .store(bytes_per_second.unwrap_or(0), Ordering::Relaxed);
    }

    /// Waits until these bytes fit the limit. The bytes are counted first and the wait
    /// comes after, so several connections sharing a limit add up to it exactly.
    pub async fn take(&self, bytes: u32) {
        let rate = self.bytes_per_second.load(Ordering::Relaxed);
        if rate == 0 {
            return;
        }
        let rate = f64::from(rate);
        let wait = {
            let mut balance = self.balance.lock();
            let now = Instant::now();
            let earned = now.duration_since(balance.updated).as_secs_f64() * rate;
            let burst = (rate / 4.0).max(MIN_BURST);
            balance.bytes = (balance.bytes + earned).min(burst) - f64::from(bytes);
            balance.updated = now;
            (balance.bytes < 0.0).then(|| Duration::from_secs_f64(-balance.bytes / rate))
        };
        if let Some(wait) = wait {
            tokio::time::sleep(wait).await;
        }
    }
}

#[derive(Default)]
struct Received {
    bytes: u64,
    until: Option<u64>,
}

/// How far one torrent may download. The app shares its speed limits between all kinds of
/// download, so it pays for what has arrived and moves this forward. A torrent with no peers
/// costs nothing that way, and other downloads get its share.
#[derive(Default)]
pub struct Allowance {
    received: Mutex<Received>,
    moved: Notify,
}

impl Allowance {
    /// Lets the torrent receive up to `until` bytes in all, or any amount with None.
    /// Returns how many it has received so far.
    pub fn allow(&self, until: Option<u64>) -> u64 {
        let mut received = self.received.lock();
        received.until = until;
        let bytes = received.bytes;
        drop(received);
        self.moved.notify_waiters();
        bytes
    }

    /// Waits until a block may be received, and counts it.
    pub async fn take(&self, bytes: u32) {
        loop {
            let moved = self.moved.notified();
            tokio::pin!(moved);
            moved.as_mut().enable();
            {
                let mut received = self.received.lock();
                if received.until.is_none_or(|until| received.bytes < until) {
                    received.bytes += u64::from(bytes);
                    return;
                }
            }
            moved.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u32 = 1024 * 1024;

    #[tokio::test(start_paused = true)]
    async fn test_no_limit_never_waits() {
        let limiter = ByteLimiter::new(None);
        let start = Instant::now();
        limiter.take(u32::MAX).await;
        assert_eq!(Instant::now(), start);
    }

    #[tokio::test(start_paused = true)]
    async fn test_zero_means_no_limit() {
        let limiter = ByteLimiter::new(Some(0));
        let start = Instant::now();
        limiter.take(MIB).await;
        assert_eq!(Instant::now(), start);
    }

    #[tokio::test(start_paused = true)]
    async fn test_a_stream_is_held_to_the_limit() {
        let limiter = ByteLimiter::new(Some(MIB));
        let start = Instant::now();
        for _ in 0..64 {
            limiter.take(MIB / 16).await;
        }
        let seconds = (Instant::now() - start).as_secs_f64();
        assert!((3.9..=4.1).contains(&seconds), "{seconds}");
    }

    #[tokio::test(start_paused = true)]
    async fn test_a_quiet_period_only_saves_a_short_burst() {
        let limiter = ByteLimiter::new(Some(MIB));
        tokio::time::advance(Duration::from_secs(3600)).await;
        let start = Instant::now();
        limiter.take(MIB / 4).await;
        assert_eq!(Instant::now(), start, "the burst covers this");
        limiter.take(MIB).await;
        let seconds = (Instant::now() - start).as_secs_f64();
        assert!((0.9..=1.1).contains(&seconds), "{seconds}");
    }

    #[tokio::test(start_paused = true)]
    async fn test_a_limit_can_be_added_and_lifted() {
        let limiter = ByteLimiter::new(None);
        limiter.set_limit(Some(MIB));
        let start = Instant::now();
        for _ in 0..32 {
            limiter.take(MIB / 16).await;
        }
        let seconds = (Instant::now() - start).as_secs_f64();
        assert!((1.9..=2.1).contains(&seconds), "{seconds}");

        limiter.set_limit(None);
        let start = Instant::now();
        limiter.take(MIB * 64).await;
        assert_eq!(Instant::now(), start);
    }

    #[tokio::test]
    async fn test_an_allowance_starts_open_and_counts() {
        let allowance = Allowance::default();
        allowance.take(10).await;
        allowance.take(5).await;
        assert_eq!(allowance.allow(None), 15);
    }

    #[tokio::test(start_paused = true)]
    async fn test_an_allowance_holds_bytes_until_it_moves() {
        let allowance = std::sync::Arc::new(Allowance::default());
        assert_eq!(allowance.allow(Some(10)), 0);
        allowance.take(10).await;

        let held = {
            let allowance = allowance.clone();
            tokio::spawn(async move { allowance.take(10).await })
        };
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert!(!held.is_finished(), "the allowance was used up");
        assert_eq!(allowance.allow(Some(20)), 10);
        held.await.unwrap();
        assert_eq!(allowance.allow(Some(20)), 20);
    }

    #[tokio::test(start_paused = true)]
    async fn test_lifting_an_allowance_frees_whoever_waits() {
        let allowance = std::sync::Arc::new(Allowance::default());
        allowance.allow(Some(0));
        let held = {
            let allowance = allowance.clone();
            tokio::spawn(async move { allowance.take(10).await })
        };
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(!held.is_finished());
        allowance.allow(None);
        held.await.unwrap();
    }
}
