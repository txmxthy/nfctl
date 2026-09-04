use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// A UTC instant that serialises as RFC 3339, so `-o json` is readable and
/// stable rather than the `time` crate's default tuple encoding.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(#[serde(with = "time::serde::rfc3339")] OffsetDateTime);

impl Timestamp {
    #[must_use]
    pub fn new(t: OffsetDateTime) -> Self {
        Self(t)
    }

    #[must_use]
    pub fn now() -> Self {
        Self(OffsetDateTime::now_utc())
    }

    #[must_use]
    pub fn get(self) -> OffsetDateTime {
        self.0
    }

    /// Parse RFC 3339 (what Kubernetes and Numaflow write).
    pub fn parse_rfc3339(s: &str) -> Result<Self, time::error::Parse> {
        OffsetDateTime::parse(s, &Rfc3339).map(Self)
    }

    /// `None` when unix nanos are out of range.
    #[must_use]
    pub fn from_unix_nanos(nanos: i128) -> Option<Self> {
        OffsetDateTime::from_unix_timestamp_nanos(nanos)
            .ok()
            .map(Self)
    }

    /// Non-negative elapsed time from `self` to `now`; `None` if `now` is earlier.
    #[must_use]
    pub fn elapsed_until(self, now: Timestamp) -> Option<Duration> {
        (now.0 - self.0).try_into().ok()
    }
}

impl From<OffsetDateTime> for Timestamp {
    fn from(t: OffsetDateTime) -> Self {
        Self(t)
    }
}

impl fmt::Debug for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0.format(&Rfc3339) {
            Ok(s) => f.write_str(&s),
            Err(_) => write!(f, "{}", self.0.unix_timestamp()),
        }
    }
}

/// Serialise a `Duration` as whole seconds (`pause_grace: 30`).
pub mod duration_secs {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        d.as_secs().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        u64::deserialize(d).map(Duration::from_secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_is_rfc3339() {
        let t = Timestamp::parse_rfc3339("2026-01-02T03:04:05Z").unwrap();
        assert_eq!(
            serde_json::to_string(&t).unwrap(),
            "\"2026-01-02T03:04:05Z\""
        );
        let back: Timestamp = serde_json::from_str("\"2026-01-02T03:04:05Z\"").unwrap();
        assert_eq!(back, t);
    }
}
