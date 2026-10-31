//! Added by Download Engine. See ../../patch.md.

use std::time::Duration;

use crate::random::random_u32;

/// Waiting times between retries. Each wait doubles until it reaches `max`, then a random
/// extra of up to the same again is added, so that clients don't all retry at one moment.
pub struct Backoff {
    next: Duration,
    max: Duration,
}

impl Backoff {
    pub fn new(min: Duration, max: Duration) -> Self {
        Self { next: min, max }
    }

    pub fn next_delay(&mut self) -> Duration {
        let delay = self.next;
        self.next = delay.saturating_mul(2).min(self.max);
        let fraction = f64::from(random_u32()) / (f64::from(u32::MAX) + 1.0);
        delay + delay.mul_f64(fraction)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delays_double_up_to_the_limit() {
        let mut backoff = Backoff::new(Duration::from_secs(10), Duration::from_secs(60));
        for seconds in [10, 20, 40, 60, 60, 60] {
            let base = Duration::from_secs(seconds);
            let delay = backoff.next_delay();
            assert!(delay >= base && delay < base * 2, "{base:?} {delay:?}");
        }
    }

    #[test]
    fn test_delays_are_spread_out() {
        let delays: std::collections::HashSet<_> = (0..20)
            .map(|_| Backoff::new(Duration::from_secs(1), Duration::from_secs(1)).next_delay())
            .collect();
        assert!(delays.len() > 1);
    }
}