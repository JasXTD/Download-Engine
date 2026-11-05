//! When a finished torrent stops sharing on its own. The decision is a pure function of the
//! saved totals and the chosen limits, with no clock, disk or network, so it can be checked
//! alone. Whole numbers only: no rounding, overflow or NaN can move a limit.
use std::num::{NonZeroU32, NonZeroU64};

/// Share ratios offered in Preferences, in hundredths. The database accepts only these.
pub const RATIOS: [u32; 5] = [50, 100, 200, 300, 500];
/// Sharing times offered in Preferences, in seconds. The database accepts only these.
pub const TIMES: [u32; 5] = [3_600, 21_600, 86_400, 259_200, 604_800];
/// Sharing stops at ratio 2 or after a week, whichever comes first, unless changed.
pub const DEFAULT_RATIO: u32 = 200;
pub const DEFAULT_TIME: u32 = 604_800;

/// What one torrent has shared, added up across every run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shared {
    pub uploaded: u64,
    /// The chosen files' size. A torrent with nothing chosen can't be shared at all.
    pub chosen_size: NonZeroU64,
    /// Time spent sharing, measured with a steady clock while sharing runs.
    pub shared_secs: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Limits {
    pub ratio_hundredths: Option<NonZeroU32>,
    pub seconds: Option<NonZeroU32>,
}
impl Limits {
    pub fn new(ratio_hundredths: Option<u32>, seconds: Option<u32>) -> Self {
        Self {
            ratio_hundredths: ratio_hundredths.and_then(NonZeroU32::new),
            seconds: seconds.and_then(NonZeroU32::new),
        }
    }
    pub fn is_none(&self) -> bool {
        self.ratio_hundredths.is_none() && self.seconds.is_none()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reached {
    Ratio,
    Time,
}
impl Reached {
    /// How the database names it.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Ratio => "ratio",
            Self::Time => "time",
        }
    }
    pub(crate) fn parse(text: &str) -> Option<Self> {
        match text {
            "ratio" => Some(Self::Ratio),
            "time" => Some(Self::Time),
            _ => None,
        }
    }
}

/// Whether sharing has to stop now, and why. The ratio wins when both are reached.
pub fn reached(shared: Shared, limits: Limits) -> Option<Reached> {
    let ratio = limits.ratio_hundredths.is_some_and(|ratio| {
        u128::from(shared.uploaded) * 100
            >= u128::from(ratio.get()) * u128::from(shared.chosen_size.get())
    });
    let time = limits
        .seconds
        .is_some_and(|seconds| shared.shared_secs >= u64::from(seconds.get()));
    if ratio {
        Some(Reached::Ratio)
    } else if time {
        Some(Reached::Time)
    } else {
        None
    }
}

/// The share ratio in hundredths, rounded down, for showing.
pub fn ratio_hundredths(uploaded: u64, chosen_size: NonZeroU64) -> u64 {
    let ratio = u128::from(uploaded) * 100 / u128::from(chosen_size.get());
    u64::try_from(ratio).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared(uploaded: u64, size: u64, secs: u64) -> Shared {
        Shared {
            uploaded,
            chosen_size: NonZeroU64::new(size).unwrap(),
            shared_secs: secs,
        }
    }

    #[test]
    fn a_ratio_limit_fires_exactly_at_the_limit_and_not_one_byte_before() {
        let limits = Limits::new(Some(200), None);
        assert_eq!(reached(shared(1_999, 1_000, 0), limits), None);
        assert_eq!(
            reached(shared(2_000, 1_000, 0), limits),
            Some(Reached::Ratio)
        );
        assert_eq!(
            reached(shared(2_001, 1_000, 0), limits),
            Some(Reached::Ratio)
        );
        // Half a ratio on an odd size: 3 × 0.5 = 1.5 bytes, so 2 bytes are needed.
        let half = Limits::new(Some(50), None);
        assert_eq!(reached(shared(1, 3, 0), half), None);
        assert_eq!(reached(shared(2, 3, 0), half), Some(Reached::Ratio));
    }

    #[test]
    fn a_time_limit_fires_exactly_at_the_limit() {
        let limits = Limits::new(None, Some(3_600));
        assert_eq!(reached(shared(0, 1, 3_599), limits), None);
        assert_eq!(reached(shared(0, 1, 3_600), limits), Some(Reached::Time));
    }

    #[test]
    fn whichever_comes_first_stops_sharing_and_the_ratio_wins_a_tie() {
        let limits = Limits::new(Some(100), Some(60));
        assert_eq!(reached(shared(10, 100, 61), limits), Some(Reached::Time));
        assert_eq!(reached(shared(100, 100, 0), limits), Some(Reached::Ratio));
        assert_eq!(reached(shared(100, 100, 61), limits), Some(Reached::Ratio));
        assert_eq!(reached(shared(99, 100, 59), limits), None);
    }

    #[test]
    fn no_limit_never_fires_even_at_the_largest_numbers() {
        let none = Limits::default();
        assert!(none.is_none());
        assert_eq!(reached(shared(u64::MAX, 1, u64::MAX), none), None);
        // Zero means no limit, so a damaged zero can't make sharing stop at once.
        assert!(Limits::new(Some(0), Some(0)).is_none());
    }

    #[test]
    fn the_largest_sizes_do_not_overflow() {
        let limits = Limits::new(Some(500), Some(u32::MAX));
        assert_eq!(reached(shared(u64::MAX, u64::MAX, 0), limits), None);
        assert_eq!(
            reached(shared(u64::MAX, u64::MAX / 5, 0), limits),
            Some(Reached::Ratio)
        );
        assert_eq!(
            reached(shared(0, u64::MAX, u64::MAX), limits),
            Some(Reached::Time)
        );
        assert_eq!(ratio_hundredths(u64::MAX, NonZeroU64::MIN), u64::MAX);
        assert_eq!(
            ratio_hundredths(1_420, NonZeroU64::new(1_000).unwrap()),
            142
        );
    }

    #[test]
    fn every_offered_value_is_a_real_limit() {
        for ratio in RATIOS {
            assert!(Limits::new(Some(ratio), None).ratio_hundredths.is_some());
        }
        for seconds in TIMES {
            assert!(Limits::new(None, Some(seconds)).seconds.is_some());
        }
        assert!(RATIOS.contains(&DEFAULT_RATIO) && TIMES.contains(&DEFAULT_TIME));
        for reason in [Reached::Ratio, Reached::Time] {
            assert_eq!(Reached::parse(reason.as_str()), Some(reason));
        }
        assert_eq!(Reached::parse("forever"), None);
    }
}
