//! Injectable clock, so time-dependent services (login throttling, and session expiry in
//! M2-05) can be tested without sleeping.
//!
//! Two readings with different jobs:
//! - [`Clock::now`]: wall time, for timestamps that are stored or compared with stored ones.
//! - [`Clock::elapsed`]: monotonic time, for in-memory intervals (rate-limit buckets) that
//!   must not jump when the wall clock is stepped.
//!
//! Production uses [`SystemClock`]; tests share one [`ManualClock`] between the services
//! under test and move both readings forward together.

use std::fmt;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use chrono::DateTime;
use deskhaven_db::time::{self, TimestampMs};

pub trait Clock: Send + Sync + fmt::Debug {
    /// Wall time as stored in the database (unix milliseconds, UTC).
    fn now(&self) -> TimestampMs;

    /// Monotonic time since an arbitrary fixed point of this clock. Never goes backwards.
    fn elapsed(&self) -> Duration;

    /// [`Clock::now`] as unix milliseconds.
    fn now_ms(&self) -> i64 {
        self.now().timestamp_millis()
    }
}

/// A clock shared by the services of one process.
pub type SharedClock = Arc<dyn Clock>;

/// The system clocks: wall time (millisecond precision) and `Instant` since first use.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> TimestampMs {
        time::now()
    }

    fn elapsed(&self) -> Duration {
        static ANCHOR: OnceLock<Instant> = OnceLock::new();
        ANCHOR.get_or_init(Instant::now).elapsed()
    }
}

/// A clock that only moves when told to. For tests; safe to share across tasks.
/// [`Clock::elapsed`] is the wall reading as a duration (clamped at 0), so both move together.
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

    fn elapsed(&self) -> Duration {
        Duration::from_millis(u64::try_from(self.ms.load(Ordering::SeqCst)).unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_moves_only_when_told() {
        let clock = ManualClock::at(1_000);
        assert_eq!(clock.now_ms(), 1_000);
        assert_eq!(clock.elapsed(), Duration::from_millis(1_000));

        clock.advance(Duration::from_millis(1_500));
        assert_eq!(clock.now_ms(), 2_500);
        assert_eq!(clock.elapsed(), Duration::from_millis(2_500));

        clock.set(42);
        assert_eq!(clock.now_ms(), 42);
    }

    #[test]
    fn system_clock_is_close_to_now() {
        let before = chrono::Utc::now().timestamp_millis();
        let now = SystemClock.now_ms();
        assert!((before..before + 60_000).contains(&now));
    }

    #[test]
    fn system_clock_elapsed_is_monotonic() {
        let a = SystemClock.elapsed();
        let b = SystemClock.elapsed();
        assert!(b >= a);
    }
}
