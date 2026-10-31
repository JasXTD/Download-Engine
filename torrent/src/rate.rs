//! Keeps a stream of bytes under a speed limit.

use parking_lot::Mutex;
use tokio::time::{Duration, Instant};

/// A short burst is allowed, so that a small block doesn't wait for a whole time slice.
const MIN_BURST: f64 = 64.0 * 1024.0;

struct Balance {
    bytes: f64,
    updated: Instant,
}

pub struct ByteLimiter {
    bytes_per_second: Option<f64>,
    balance: Mutex<Balance>,
}

impl ByteLimiter {
    pub fn new(bytes_per_second: Option<u32>) -> Self {
        Self {
            bytes_per_second: bytes_per_second.filter(|rate| *rate > 0).map(f64::from),
            balance: Mutex::new(Balance {
                bytes: 0.0,
                updated: Instant::now(),
            }),
        }
    }

    /// Waits until these bytes fit the limit. The bytes are counted first and the wait
    /// comes after, so several connections sharing a limit add up to it exactly.
    pub async fn take(&self, bytes: u32) {
        let Some(rate) = self.bytes_per_second else {
            return;
        };
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
}
