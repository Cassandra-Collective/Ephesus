//! One time format (SDD-RUST §4.0 rule 2): RFC 3339 in UTC with exactly three
//! fractional digits, `2026-10-19T12:34:56.789Z`, and the same instant in
//! compact form, `20261019T123456789Z`, wherever a time is part of a name.
//!
//! Contract: a [`Timestamp`] holds millisecond precision and nothing finer, so
//! a value written and read back is the same value. Parsing is strict: a time
//! in any other shape (an offset, missing milliseconds, extra digits) is
//! refused, because one shape everywhere is the point of the rule.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An instant, to the millisecond.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(jiff::Timestamp);

/// A text that is not a time in the one format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadTime(pub String);

impl fmt::Display for BadTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` is not a time in the one format, RFC 3339 UTC with milliseconds (2026-10-19T12:34:56.789Z)",
            self.0
        )
    }
}

impl Timestamp {
    /// The earliest instant the format writes: 0000-01-01T00:00:00.000Z, its
    /// first four-digit year.
    pub const MIN_MILLIS: i64 = -62_167_219_200_000;
    /// The latest instant the format writes: 9999-12-30T22:00:00.000Z, the last
    /// millisecond `jiff::Timestamp::from_millisecond` accepts (999 below
    /// `jiff::Timestamp::MAX`, measured 2026-10-20; a test pins it).
    pub const MAX_MILLIS: i64 = 253_402_207_200_000;

    /// The instant `millis` milliseconds after the Unix epoch, or `None`
    /// outside [`Timestamp::MIN_MILLIS`] to [`Timestamp::MAX_MILLIS`]. The format
    /// writes four-digit years only, so any earlier instant would render text the
    /// next read refuses (review finding F7).
    pub fn from_millis(millis: i64) -> Option<Timestamp> {
        if !(Self::MIN_MILLIS..=Self::MAX_MILLIS).contains(&millis) {
            return None;
        }
        jiff::Timestamp::from_millisecond(millis)
            .ok()
            .map(Timestamp)
    }

    /// Now, truncated to the millisecond. Side effect: reads the system clock.
    pub fn now() -> Timestamp {
        let millis = jiff::Timestamp::now().as_millisecond();
        // The current time is always inside years 0000 to 9999.
        Timestamp::from_millis(millis).unwrap_or(Timestamp(jiff::Timestamp::UNIX_EPOCH))
    }

    /// Milliseconds since the Unix epoch.
    pub fn as_millis(self) -> i64 {
        self.0.as_millisecond()
    }

    /// The compact form used inside names: `20261019T123456789Z`.
    pub fn compact(self) -> String {
        let text = self.to_string();
        // `2026-10-19T12:34:56.789Z` → `20261019T123456789Z`
        text.chars()
            .filter(|c| !matches!(c, '-' | ':' | '.'))
            .collect()
    }

    /// Parses the compact form, as strictly as [`FromStr`] parses the long one.
    pub fn parse_compact(text: &str) -> Result<Timestamp, BadTime> {
        let bytes = text.as_bytes();
        let shape_ok = bytes.len() == 19
            && bytes[8] == b'T'
            && bytes[18] == b'Z'
            && bytes[..8].iter().all(u8::is_ascii_digit)
            && bytes[9..18].iter().all(u8::is_ascii_digit);
        if !shape_ok {
            return Err(BadTime(text.to_string()));
        }
        let long = format!(
            "{}-{}-{}T{}:{}:{}.{}Z",
            &text[0..4],
            &text[4..6],
            &text[6..8],
            &text[9..11],
            &text[11..13],
            &text[13..15],
            &text[15..18]
        );
        long.parse().map_err(|_| BadTime(text.to_string()))
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let civil = self.0.to_zoned(jiff::tz::TimeZone::UTC).datetime();
        write!(
            f,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
            civil.year(),
            civil.month(),
            civil.day(),
            civil.hour(),
            civil.minute(),
            civil.second(),
            civil.millisecond()
        )
    }
}

impl FromStr for Timestamp {
    type Err = BadTime;

    fn from_str(text: &str) -> Result<Timestamp, BadTime> {
        let parsed: jiff::Timestamp = text.parse().map_err(|_| BadTime(text.to_string()))?;
        let value =
            Timestamp::from_millis(parsed.as_millisecond()).ok_or(BadTime(text.to_string()))?;
        // Strict: the text must be exactly the one rendering of its instant.
        if value.to_string() == text {
            Ok(value)
        } else {
            Err(BadTime(text.to_string()))
        }
    }
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Timestamp, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse()
            .map_err(|error: BadTime| serde::de::Error::custom(error))
    }
}

/// A length of time, written for a person to read: `90s`, `5m`, `6h`, `1h 30m`.
///
/// Contract: parsed with jiff's friendly format and rendered back the same way,
/// so a value round-trips. A duration is never negative here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Duration(jiff::SignedDuration);

impl Duration {
    /// A duration of `millis` milliseconds, or `None` if negative.
    pub fn from_millis(millis: i64) -> Option<Duration> {
        (millis >= 0).then(|| Duration(jiff::SignedDuration::from_millis(millis)))
    }

    /// Whole milliseconds.
    pub fn as_millis(self) -> i128 {
        self.0.as_millis()
    }
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#}", self.0)
    }
}

impl FromStr for Duration {
    type Err = BadDuration;

    fn from_str(text: &str) -> Result<Duration, BadDuration> {
        let value: jiff::SignedDuration =
            text.parse().map_err(|_| BadDuration(text.to_string()))?;
        if value.is_negative() {
            return Err(BadDuration(text.to_string()));
        }
        Ok(Duration(value))
    }
}

/// A text that is not a duration Ephesus reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadDuration(pub String);

impl fmt::Display for BadDuration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` is not a duration; write one like 90s, 5m, 6h or 1h 30m",
            self.0
        )
    }
}

impl Serialize for Duration {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Duration {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse()
            .map_err(|error: BadDuration| serde::de::Error::custom(error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_time_renders_in_the_one_format_and_round_trips() {
        let t = Timestamp::from_millis(1_760_877_296_789).unwrap();
        assert_eq!(t.to_string(), "2025-10-19T12:34:56.789Z");
        assert_eq!(t.to_string().parse::<Timestamp>(), Ok(t));
        assert_eq!(
            Timestamp::from_millis(0).unwrap().to_string(),
            "1970-01-01T00:00:00.000Z"
        );
    }

    #[test]
    fn every_instant_it_makes_renders_text_it_reads_back() {
        // Review finding F7: a negative year rendered as `-001…`, which the
        // strict reader refuses, so `render` could write a file `parse` rejects.
        for millis in [Timestamp::MIN_MILLIS, 0, Timestamp::MAX_MILLIS] {
            let t = Timestamp::from_millis(millis).unwrap();
            assert_eq!(t.to_string().parse::<Timestamp>(), Ok(t), "{millis}");
            assert_eq!(Timestamp::parse_compact(&t.compact()), Ok(t), "{millis}");
        }
        assert_eq!(
            Timestamp::from_millis(Timestamp::MIN_MILLIS)
                .unwrap()
                .to_string(),
            "0000-01-01T00:00:00.000Z"
        );
        assert_eq!(
            Timestamp::from_millis(Timestamp::MAX_MILLIS)
                .unwrap()
                .to_string(),
            "9999-12-30T22:00:00.000Z"
        );
        assert!(
            jiff::Timestamp::from_millisecond(Timestamp::MAX_MILLIS).is_ok()
                && jiff::Timestamp::from_millisecond(Timestamp::MAX_MILLIS + 1).is_err(),
            "the upper bound is jiff's own"
        );
        assert_eq!(Timestamp::from_millis(Timestamp::MIN_MILLIS - 1), None);
        assert_eq!(Timestamp::from_millis(Timestamp::MAX_MILLIS + 1), None);
        assert_eq!(Timestamp::from_millis(i64::MIN), None);
    }

    #[test]
    fn any_other_time_shape_is_refused() {
        for text in [
            "2025-10-19T12:34:56Z",
            "2025-10-19T12:34:56.7Z",
            "2025-10-19T12:34:56.789123Z",
            "2025-10-19T12:34:56.789+00:00",
            "2025-10-19T14:34:56.789+02:00",
            "2025-10-19 12:34:56.789Z",
            "2025-10-19T12:34:56.789z",
            "yesterday",
            "",
        ] {
            assert!(text.parse::<Timestamp>().is_err(), "{text}");
        }
    }

    #[test]
    fn the_compact_form_is_the_same_instant_and_round_trips() {
        let t = Timestamp::from_millis(1_760_877_296_789).unwrap();
        assert_eq!(t.compact(), "20251019T123456789Z");
        assert_eq!(Timestamp::parse_compact("20251019T123456789Z"), Ok(t));
        for bad in [
            "20251019T12345678Z",
            "20251019X123456789Z",
            "2025101aT123456789Z",
            "20251319T123456789Z",
        ] {
            assert!(Timestamp::parse_compact(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn now_has_no_precision_finer_than_a_millisecond() {
        let t = Timestamp::now();
        assert_eq!(t.to_string().parse::<Timestamp>(), Ok(t));
    }

    #[test]
    fn a_duration_reads_the_way_a_person_writes_it_and_round_trips() {
        for (text, millis) in [
            ("90s", 90_000),
            ("5m", 300_000),
            ("6h", 21_600_000),
            ("1h 30m", 5_400_000),
        ] {
            let d: Duration = text.parse().unwrap();
            assert_eq!(d.as_millis(), millis, "{text}");
            assert_eq!(d.to_string().parse::<Duration>(), Ok(d), "{text}");
        }
        assert!("-5m".parse::<Duration>().is_err());
        assert!("soon".parse::<Duration>().is_err());
        assert_eq!(Duration::from_millis(-1), None);
    }

    #[test]
    fn bad_values_say_what_is_allowed() {
        assert!(
            BadTime("x".into())
                .to_string()
                .contains("2026-10-19T12:34:56.789Z")
        );
        assert!(BadDuration("x".into()).to_string().contains("90s"));
    }

    /// Round 1 (2026-10-20): `Duration::from_millis` was tested only for the
    /// value it refuses.
    #[test]
    fn a_duration_is_made_from_whole_milliseconds() {
        let d = Duration::from_millis(90_000).unwrap();
        assert_eq!(d.as_millis(), 90_000);
        assert_eq!(d, "90s".parse::<Duration>().unwrap());
        assert_eq!(Duration::from_millis(0).map(Duration::as_millis), Some(0));
    }
}
