//! Audit rate limits (06 §2.4: by source IP and by device id).
//!
//! - **Per client network** ([`AUDIT_PER_IP_PER_MINUTE`]): the login limiter's buckets (IPv4
//!   /32, IPv6 /64, shared proxy-fallback bucket) with a quota of their own; every request is
//!   charged, malformed and unknown-device ones included.
//! - **Per device** ([`AUDIT_PER_DEVICE_PER_MINUTE`]): charged only after the device is
//!   verified, so records under a stolen id with a wrong uuid cannot spend its quota.
//!
//! Over either the endpoint answers 429; the client retries the record (10 s, 30 s) with the
//! same nonce. Both run on the monotonic clock; buckets are in memory and per process.

use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use governor::middleware::NoOpMiddleware;
use governor::state::keyed::HashMapStateStore;
use governor::{Quota, RateLimiter};

use crate::clock::SharedClock;

/// Audit requests per client network per minute (burst the same). A device posts three
/// connection records per connection plus file and alarm records; this lets an office NAT
/// open a few hundred connections a minute.
pub const AUDIT_PER_IP_PER_MINUTE: u32 = 900;
/// Records per verified device per minute (burst the same): a busy device with bursts of
/// file jobs or failed-login alarms stays below it.
pub const AUDIT_PER_DEVICE_PER_MINUTE: u32 = 300;
/// Checks between two sweeps of refilled buckets (which are dropped).
const SWEEP_EVERY: u64 = 1024;

/// governor's view of the injected clock: its monotonic reading.
#[derive(Clone)]
struct Monotonic(SharedClock);

impl governor::clock::Clock for Monotonic {
    type Instant = Duration;

    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

type Buckets = RateLimiter<String, HashMapStateStore<String>, Monotonic, NoOpMiddleware<Duration>>;

/// Token bucket per device RustDesk id. Bounded by the devices seen within a minute (refilled
/// buckets are swept), and only verified devices get one.
pub(super) struct DeviceLimit {
    clock: Monotonic,
    buckets: Buckets,
    checks: AtomicU64,
}

impl DeviceLimit {
    pub fn new(clock: SharedClock) -> Self {
        let clock = Monotonic(clock);
        let per_minute = NonZeroU32::new(AUDIT_PER_DEVICE_PER_MINUTE).unwrap_or(NonZeroU32::MIN);
        let buckets = RateLimiter::new(
            Quota::per_minute(per_minute),
            HashMapStateStore::default(),
            clock.clone(),
        );
        Self {
            clock,
            buckets,
            checks: AtomicU64::new(0),
        }
    }

    /// Takes one token of `rustdesk_id`, or returns how long until one is free.
    pub fn check(&self, rustdesk_id: &str) -> Result<(), Duration> {
        let now = governor::clock::Clock::now(&self.clock);
        let result = self
            .buckets
            .check_key(&rustdesk_id.to_owned())
            .map_err(|not_until| not_until.wait_time_from(now));
        if self.checks.fetch_add(1, Ordering::Relaxed) % SWEEP_EVERY == SWEEP_EVERY - 1 {
            self.buckets.retain_recent();
            self.buckets.shrink_to_fit();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::clock::ManualClock;

    #[test]
    fn each_device_has_its_own_quota_refilled_on_the_monotonic_clock() {
        let clock = Arc::new(ManualClock::at(1_000_000));
        let limit = DeviceLimit::new(clock.clone());
        for _ in 0..AUDIT_PER_DEVICE_PER_MINUTE {
            assert!(limit.check("a").is_ok());
        }

        let over = limit.check("a");
        let other = limit.check("b");
        clock.step_wall(-3_600_000);
        clock.advance(Duration::from_secs(1));
        let refilled = limit.check("a");

        assert!(over.is_err_and(|wait| wait > Duration::ZERO));
        assert!(other.is_ok());
        assert!(refilled.is_ok(), "one token per 200 ms");
    }
}
