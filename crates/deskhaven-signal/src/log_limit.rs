//! Rate limit for log lines of events a client can trigger at will (deskhaven code, not
//! vendored).

use std::{
    sync::{Mutex, PoisonError},
    time::{Duration, Instant},
};

/// Counts events and says when to log them: the first one, then at most once per `every`,
/// with the number of events since the last line.
pub(crate) struct LogLimit {
    every: Duration,
    /// Events since the last line, and when that line was logged.
    state: Mutex<(u64, Option<Instant>)>,
}

impl LogLimit {
    pub(crate) const fn new(every: Duration) -> Self {
        Self {
            every,
            state: Mutex::new((0, None)),
        }
    }

    /// Records one event at `now`; `Some(count)` if a line should be logged now.
    pub(crate) fn hit(&self, now: Instant) -> Option<u64> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let (count, last) = &mut *state;
        *count += 1;
        if last.is_some_and(|t| now.saturating_duration_since(t) < self.every) {
            return None;
        }
        *last = Some(now);
        Some(std::mem::take(count))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_the_first_hit_then_once_per_interval_with_the_count() {
        let limit = LogLimit::new(Duration::from_secs(60));
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);

        assert_eq!(limit.hit(at(0)), Some(1));
        assert_eq!(limit.hit(at(1)), None);
        assert_eq!(limit.hit(at(59)), None);
        assert_eq!(limit.hit(at(60)), Some(3));
        assert_eq!(limit.hit(at(61)), None);
        assert_eq!(limit.hit(at(200)), Some(2));
    }
}
