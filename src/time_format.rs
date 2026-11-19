//! Timestamp validation at persisted diagnostic boundaries.
use anyhow::{Result, ensure};
use jiff::{Timestamp, tz::TimeZone};

pub struct DiagnosticTimestamp {
    instant: Timestamp,
    // Jiff maps an RFC3339 leap second to :59. Retain the distinction from
    // ordinary :59 records when displaying and sorting existing logs.
    leap_second: bool,
}

impl DiagnosticTimestamp {
    pub(crate) fn sort_key(&self) -> (i128, bool, Timestamp) {
        let second = self.instant.as_nanosecond().div_euclid(1_000_000_000);
        (second, self.leap_second, self.instant)
    }

    pub fn display_in(&self, zone: TimeZone) -> String {
        let format = if self.leap_second {
            "%d %b %Y %H:%M:60%.3f"
        } else {
            "%d %b %Y %H:%M:%S%.3f"
        };
        self.instant
            .to_zoned_datetime(zone)
            .strftime(format)
            .to_string()
    }
}

/// Keep diagnostics restricted to RFC3339. Jiff's general parser also accepts
/// shortened times and bracketed annotations, which must not carry arbitrary
/// text into reports that promise to contain only typed metadata.
pub fn parse_rfc3339(value: &str) -> Result<DiagnosticTimestamp> {
    let bytes = value.as_bytes();
    ensure!(
        bytes.len() >= 20 && value.is_ascii(),
        "Invalid RFC3339 timestamp"
    );
    for (index, byte) in bytes[..19].iter().enumerate() {
        let valid = match index {
            4 | 7 => *byte == b'-',
            10 => matches!(*byte, b'T' | b't' | b' '),
            13 | 16 => *byte == b':',
            _ => byte.is_ascii_digit(),
        };
        ensure!(valid, "Invalid RFC3339 timestamp");
    }
    let mut offset = &bytes[19..];
    if offset.first() == Some(&b'.') {
        let digits = offset[1..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        ensure!(digits > 0, "Invalid RFC3339 fraction");
        offset = &offset[1 + digits..];
    }
    ensure!(
        matches!(offset, [b'Z' | b'z'])
            || (offset.len() == 6
                && matches!(offset[0], b'+' | b'-')
                && offset[3] == b':'
                && offset[1..3]
                    .iter()
                    .chain(&offset[4..6])
                    .all(u8::is_ascii_digit)
                && (offset[1] - b'0') * 10 + (offset[2] - b'0') < 24
                && (offset[4] - b'0') * 10 + (offset[5] - b'0') < 60),
        "Invalid RFC3339 offset"
    );
    Ok(DiagnosticTimestamp {
        instant: value.parse()?,
        leap_second: &bytes[17..19] == b"60",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_precision_and_legacy_offsets_remain_compatible() {
        let time = Timestamp::new(1_793_491_200, 123_456_789).unwrap();
        assert_eq!(format!("{time:.0}"), "2026-11-01T00:00:00Z");
        assert_eq!(format!("{time:.3}"), "2026-11-01T00:00:00.123Z");
        for value in [
            "2026-11-01T00:00:00.123456789Z",
            "2026-11-01T08:00:00.123456789+08:00",
            "2026-10-31t20:00:00.123456789-04:00",
        ] {
            assert_eq!(parse_rfc3339(value).unwrap().instant, time, "{value}");
        }
        assert_eq!(
            parse_rfc3339("1970-01-01 00:00:00+00:00").unwrap().instant,
            Timestamp::UNIX_EPOCH
        );
    }

    #[test]
    fn leap_second_keeps_its_label_and_sorts_between_neighboring_seconds() {
        let before = parse_rfc3339("2016-12-31T23:59:59.999Z").unwrap();
        let leap = parse_rfc3339("2017-01-01T00:59:60.123+01:00").unwrap();
        let after = parse_rfc3339("2017-01-01T00:00:00Z").unwrap();
        assert!(before.sort_key() < leap.sort_key() && leap.sort_key() < after.sort_key());
        assert_eq!(leap.display_in(TimeZone::UTC), "31 Dec 2016 23:59:60.123");
        assert_eq!(after.display_in(TimeZone::UTC), "01 Jan 2017 00:00:00.000");
    }

    #[test]
    fn diagnostic_timestamps_reject_annotations_shortcuts_and_invalid_values() {
        for value in [
            "2026-11-01T00Z",
            "2026-11-01T00:00Z",
            "2026-11-01T00:00:00",
            "2026-11-01T00:00:00Z[secret]",
            "2026-11-01T00:00:00Z ",
            "2026-11-01T00:00:00.Z",
            "2026-11-01T00:00:00+08",
            "2026-11-01T00:00:00+24:00",
            "2026-11-01T00:00:00+23:60",
            "2026-02-30T00:00:00Z",
            "invalid",
            "日期",
        ] {
            assert!(parse_rfc3339(value).is_err(), "{value}");
        }
    }
}
