//! Timestamp convention: every time column is `BIGINT` unix milliseconds (UTC).
//!
//! Entities declare such columns as [`TimestampMs`]; it reads/writes `i64` millis and
//! derefs to `chrono::DateTime<Utc>`.
//!
//! Build values only via [`now`] or [`from_millis`]. Wrapping an arbitrary `DateTime` keeps
//! its sub-millisecond part, which the DB drops, so the stored row would no longer compare
//! equal to the in-memory value.

use chrono::{DateTime, SubsecRound, Utc};

pub use sea_orm::prelude::ChronoUnixTimestampMillis as TimestampMs;

const MILLIS_DIGITS: u16 = 3;

/// Current time truncated to millisecond precision, so it round-trips through the DB unchanged.
pub fn now() -> TimestampMs {
    TimestampMs(Utc::now().trunc_subsecs(MILLIS_DIGITS))
}

/// Timestamp from unix milliseconds; `None` when out of chrono's range.
pub fn from_millis(ms: i64) -> Option<TimestampMs> {
    DateTime::<Utc>::from_timestamp_millis(ms).map(TimestampMs)
}
