//! Rate limits of `POST /api/sysinfo` and `POST /api/heartbeat` (06 §2.4: by source IP and
//! by device). The sysinfo rules:
//!
//! - **Per client network** ([`SYSINFO_PER_IP_PER_MINUTE`]): the login limiter's buckets
//!   (IPv4 /32, IPv6 /64, shared proxy-fallback bucket) with a quota of their own, never the
//!   login budget. A real client uploads once per start or username change, so an office
//!   behind one NAT stays far below it.
//! - **Per device** ([`SYSINFO_WRITE_INTERVAL`]): at most one *changing* write per id per
//!   interval; unchanged uploads (no write) are not limited.
//!
//! Over either limit the endpoint answers 429. The client treats any reply other than
//! `SYSINFO_UPDATED` / `ID_NOT_FOUND` as "retry in 120 s" (`sync.rs:221-228`), which is past
//! both windows, so a real client never loops and its next upload is stored. Answering
//! `SYSINFO_UPDATED` without writing instead would make it stop uploading and silently keep
//! stale data until its next restart.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use crate::clock::SharedClock;

/// Sysinfo requests per client network per minute (burst the same).
pub const SYSINFO_PER_IP_PER_MINUTE: u32 = 120;
/// Heartbeat requests that need a database lookup (malformed, unknown id, identity not cached,
/// uuid mismatch) per client network per minute (burst the same), the same buckets as
/// sysinfo with a quota of their own. Heartbeats of known devices cost no lookup and are
/// not charged (they have [`HEARTBEAT_MIN_INTERVAL`]), so a large NAT of verified devices is
/// never throttled as a whole; this caps what unknown ids can cost from one network.
pub const HEARTBEAT_PER_IP_PER_MINUTE: u32 = 6_000;
/// Minimum time between two accepted heartbeats of one verified device (06 §2.4). The
/// client sends at most every 3 s; a refused heartbeat (429) loses nothing, the next one
/// carries the same strategy and disconnects. Checked after verification, so a stranger
/// cannot use a device's id to starve it.
pub const HEARTBEAT_MIN_INTERVAL: Duration = Duration::from_secs(2);
/// Minimum time between two changing sysinfo writes of one device.
pub const SYSINFO_WRITE_INTERVAL: Duration = Duration::from_secs(30);
/// Expired entries are dropped once the table holds more than this.
const PRUNE_ABOVE: usize = 10_000;

/// Last changing write per device (monotonic ms, [`Clock::elapsed_ms`](crate::clock::Clock::elapsed_ms)).
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

    /// `true` (and the write counts from now) unless `rustdesk_id` wrote within the interval.
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

    /// Forgets `rustdesk_id` (its device was deleted).
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

    /// The interval runs on the monotonic clock: a wall clock stepped back does not block
    /// writes until it catches up.
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
