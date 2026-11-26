//! Two readings: [`Clock::now`] (wall) for stored timestamps, [`Clock::elapsed`] (monotonic) for
//! in-memory intervals that must not jump when the wall clock is stepped.

use std::fmt;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use chrono::DateTime;
use deskhaven_db::time::{self, TimestampMs};

pub trait Clock: Send + Sync + fmt::Debug {
    fn now(&self) -> TimestampMs;

    /// Never goes backwards.
    fn elapsed(&self) -> Duration;

    fn now_ms(&self) -> i64 {
        self.now().timestamp_millis()
    }

    fn elapsed_ms(&self) -> i64 {
        i64::try_from(self.elapsed().as_millis()).unwrap_or(i64::MAX)
    }
}

pub type SharedClock = Arc<dyn Clock>;

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

/// Both readings move together unless [`ManualClock::step_wall`] moved the wall reading alone.
#[derive(Debug)]
pub struct ManualClock {
    ms: AtomicI64,
    wall_offset: AtomicI64,
}

impl ManualClock {
    pub fn at(ms: i64) -> Self {
        Self {
            ms: AtomicI64::new(ms),
            wall_offset: AtomicI64::new(0),
        }
    }

    pub fn advance(&self, by: Duration) {
        let by = i64::try_from(by.as_millis()).unwrap_or(i64::MAX);
        self.ms.fetch_add(by, Ordering::SeqCst);
    }

    pub fn set(&self, ms: i64) {
        self.ms.store(ms, Ordering::SeqCst);
        self.wall_offset.store(0, Ordering::SeqCst);
    }

    pub fn step_wall(&self, by_ms: i64) {
        self.wall_offset.fetch_add(by_ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> TimestampMs {
        let ms = self.ms.load(Ordering::SeqCst) + self.wall_offset.load(Ordering::SeqCst);
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
    fn a_wall_step_moves_only_the_wall_reading() {
        let clock = ManualClock::at(10_000);

        clock.step_wall(-4_000);
        clock.advance(Duration::from_millis(500));

        assert_eq!(clock.now_ms(), 6_500);
        assert_eq!(clock.elapsed(), Duration::from_millis(10_500));
        assert_eq!(clock.elapsed_ms(), 10_500);
    }
}
