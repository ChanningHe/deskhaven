//! Over either limit the endpoint answers 429: the client retries in 120 s, past both windows.
//! Answering `SYSINFO_UPDATED` without writing would make it stop uploading and keep stale data
//! until its next restart.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use crate::clock::SharedClock;

pub const SYSINFO_PER_IP_PER_MINUTE: u32 = 120;
/// Heartbeats of known devices cost no lookup and are not charged, so a large NAT of verified
/// devices is never throttled as a whole.
pub const HEARTBEAT_PER_IP_PER_MINUTE: u32 = 6_000;
/// Checked after verification, so a stranger cannot use a device's id to starve it; a refused
/// heartbeat loses nothing.
pub const HEARTBEAT_MIN_INTERVAL: Duration = Duration::from_secs(2);
pub const SYSINFO_WRITE_INTERVAL: Duration = Duration::from_secs(30);
const PRUNE_ABOVE: usize = 10_000;

pub(super) struct WriteLimit {
    clock: SharedClock,
    last: Mutex<HashMap<String, i64>>,
}

impl WriteLimit {
    pub fn new(clock: SharedClock) -> Self {
        Self {
            clock,
            last: Mutex::default(),
        }
    }

    pub fn try_acquire(&self, rustdesk_id: &str) -> bool {
        let now = self.clock.elapsed_ms();
        let interval = i64::try_from(SYSINFO_WRITE_INTERVAL.as_millis()).unwrap_or(i64::MAX);
        let recent = |at: i64| now.saturating_sub(at) < interval;
        let mut last = self.lock();
        if last.get(rustdesk_id).copied().is_some_and(recent) {
            return false;
        }
        if last.len() >= PRUNE_ABOVE {
            last.retain(|_, at| recent(*at));
        }
        last.insert(rustdesk_id.to_owned(), now);
        true
    }

    pub fn forget(&self, rustdesk_id: &str) {
        self.lock().remove(rustdesk_id);
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, i64>> {
        // Map operations cannot leave it inconsistent.
        self.last.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::clock::ManualClock;

    #[test]
    fn one_write_per_device_per_interval() {
        let clock = Arc::new(ManualClock::at(1_000_000));
        let limit = WriteLimit::new(clock.clone());

        assert!(limit.try_acquire("a"));
        assert!(!limit.try_acquire("a"), "within the interval");
        assert!(limit.try_acquire("b"), "per device");
        clock.advance(SYSINFO_WRITE_INTERVAL - Duration::from_millis(1));
        assert!(!limit.try_acquire("a"));
        clock.advance(Duration::from_millis(1));
        assert!(limit.try_acquire("a"), "interval elapsed");
        limit.forget("b");
        assert!(limit.try_acquire("b"), "forgotten");
    }

    #[test]
    fn a_wall_clock_step_back_does_not_extend_the_interval() {
        let clock = Arc::new(ManualClock::at(1_000_000));
        let limit = WriteLimit::new(clock.clone());
        assert!(limit.try_acquire("a"));

        clock.step_wall(-3_600_000);
        clock.advance(SYSINFO_WRITE_INTERVAL);

        assert!(limit.try_acquire("a"));
    }
}
