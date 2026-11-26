//! Every time column is `BIGINT` unix milliseconds (UTC). Build values only via [`now`] or
//! [`from_millis`]: a wrapped `DateTime` keeps its sub-millisecond part, which the DB drops.

use chrono::{DateTime, SubsecRound, Utc};

pub use sea_orm::prelude::ChronoUnixTimestampMillis as TimestampMs;

const MILLIS_DIGITS: u16 = 3;

/// Truncated to milliseconds so it round-trips through the DB unchanged.
pub fn now() -> TimestampMs {
    TimestampMs(Utc::now().trunc_subsecs(MILLIS_DIGITS))
}

pub fn from_millis(ms: i64) -> Option<TimestampMs> {
    DateTime::<Utc>::from_timestamp_millis(ms).map(TimestampMs)
}
