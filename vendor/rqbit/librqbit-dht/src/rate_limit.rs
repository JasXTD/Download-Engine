//! Added by Download Engine. See ../../patch.md.

use std::time::Duration;

use parking_lot::Mutex;
use tokio::time::Instant;

const QUERIES_PER_SECOND: u32 = 250;
const REFILL_INTERVAL: Duration = Duration::from_millis(100);
const REFILL_AMOUNT: u32 = QUERIES_PER_SECOND / 10;

struct Tokens {
    available: u32,
    refilled_at: Instant,
}

/// Limits outgoing DHT queries, so a torrent can't flood the network or the user's router.
/// A quiet period saves up at most one second of queries. Nothing is held while waiting,
/// so a cancelled query costs nothing.
pub(crate) struct RateLimiter {
    tokens: Mutex<Tokens>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            tokens: Mutex::new(Tokens {
                available: REFILL_AMOUNT,
                refilled_at: Instant::now(),
            }),
        }
    }

    pub async fn acquire_one(&self) {
        while let Some(wait) = self.try_acquire() {
            tokio::time::sleep(wait).await;
        }
    }

    /// Takes a token, or says how long until the next refill.
    fn try_acquire(&self) -> Option<Duration> {
        let mut tokens = self.tokens.lock();
        let now = Instant::now();
        let elapsed = now.duration_since(tokens.refilled_at);
        let refills =
            u32::try_from(elapsed.as_millis() / REFILL_INTERVAL.as_millis()).unwrap_or(u32::MAX);
        tokens.available = tokens
            .available
            .saturating_add(refills.saturating_mul(REFILL_AMOUNT))
            .min(QUERIES_PER_SECOND);
        tokens.refilled_at += REFILL_INTERVAL * refills;
        if tokens.available > 0 {
            tokens.available -= 1;
            return None;
        }
        Some(tokens.refilled_at + REFILL_INTERVAL - now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{advance, timeout};

    #[tokio::test(start_paused = true)]
    async fn test_starts_with_one_refill() {
        let limiter = RateLimiter::new();
        let start = Instant::now();
        for _ in 0..REFILL_AMOUNT {
            limiter.acquire_one().await;
        }
        assert_eq!(Instant::now(), start);
        limiter.acquire_one().await;
        assert_eq!(Instant::now() - start, REFILL_INTERVAL);
    }

    #[tokio::test(start_paused = true)]
    async fn test_sustained_rate() {
        let limiter = RateLimiter::new();
        let start = Instant::now();
        // The first refill is free, so one second later 25 + 250 queries have gone out.
        for _ in 0..REFILL_AMOUNT + QUERIES_PER_SECOND {
            limiter.acquire_one().await;
        }
        assert_eq!(Instant::now() - start, Duration::from_secs(1));
    }

    #[tokio::test(start_paused = true)]
    async fn test_a_quiet_period_saves_at_most_one_second() {
        let limiter = RateLimiter::new();
        advance(Duration::from_secs(3600)).await;
        let start = Instant::now();
        for _ in 0..QUERIES_PER_SECOND {
            limiter.acquire_one().await;
        }
        assert_eq!(Instant::now(), start);
        limiter.acquire_one().await;
        assert_eq!(Instant::now() - start, REFILL_INTERVAL);
    }

    #[tokio::test(start_paused = true)]
    async fn test_a_cancelled_wait_costs_nothing() {
        let limiter = RateLimiter::new();
        for _ in 0..REFILL_AMOUNT {
            limiter.acquire_one().await;
        }
        let short = REFILL_INTERVAL / 2;
        assert!(timeout(short, limiter.acquire_one()).await.is_err());
        advance(short).await;
        for _ in 0..REFILL_AMOUNT {
            limiter.acquire_one().await;
        }
        assert!(timeout(short, limiter.acquire_one()).await.is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn test_waiting_queries_share_the_tokens() {
        let limiter = RateLimiter::new();
        let start = Instant::now();
        let queries = (0..REFILL_AMOUNT * 3).map(|_| limiter.acquire_one());
        let mut queries: Vec<_> = queries.map(Box::pin).collect();
        for query in &mut queries {
            query.as_mut().await;
        }
        assert_eq!(Instant::now() - start, REFILL_INTERVAL * 2);
    }
}
