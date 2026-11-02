//! Injectable wall clock, so time-dependent services (login lockout, and session expiry in
//! M2-05) can be tested without sleeping.
//!
//! Production uses [`SystemClock`]; tests share one [`ManualClock`] between the services
//! under test and move it forward explicitly.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use chrono::DateTime;
use deskhaven_db::time::{self, TimestampMs};

/// Source of "now" as stored in the database (unix milliseconds, UTC).
pub trait Clock: Send + Sync + fmt::Debug {
    fn now(&self) -> TimestampMs;

    /// [`Clock::now`] as unix milliseconds.
    fn now_ms(&self) -> i64 {
        self.now().timestamp_millis()
    }
}

/// A clock shared by the services of one process.
pub type SharedClock = Arc<dyn Clock>;

/// The system wall clock (millisecond precision).
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> TimestampMs {
        time::now()
    }
}

/// A clock that only moves when told to. For tests; safe to share across tasks.
#[derive(Debug)]
pub struct ManualClock {
    ms: AtomicI64,
}

impl ManualClock {
    /// A clock reading `ms` unix milliseconds.
    pub fn at(ms: i64) -> Self {
        Self {
            ms: AtomicI64::new(ms),
        }
    }

    pub fn advance(&self, by: Duration) {
        let by = i64::try_from(by.as_millis()).unwrap_or(i64::MAX);
        self.ms.fetch_add(by, Ordering::SeqCst);
    }

    pub fn set(&self, ms: i64) {
        self.ms.store(ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> TimestampMs {
        let ms = self.ms.load(Ordering::SeqCst);
        // Out of chrono's range only if a test sets an absurd value.
        time::from_millis(ms).unwrap_or(TimestampMs(DateTime::UNIX_EPOCH))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_moves_only_when_told() {
        let clock = ManualClock::at(1_000);
        assert_eq!(clock.now_ms(), 1_000);

        clock.advance(Duration::from_millis(1_500));
        assert_eq!(clock.now_ms(), 2_500);

        clock.set(42);
        assert_eq!(clock.now_ms(), 42);
    }

    #[test]
    fn system_clock_is_close_to_now() {
        let before = chrono::Utc::now().timestamp_millis();
        let now = SystemClock.now_ms();
        assert!((before..before + 60_000).contains(&now));
    }
}
