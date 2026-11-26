use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use governor::middleware::NoOpMiddleware;
use governor::state::keyed::HashMapStateStore;
use governor::{Quota, RateLimiter};

use crate::clock::SharedClock;

pub const WEB_SESSIONS_PER_USER_PER_MINUTE: u32 = 10;
/// Each makes the device set up a relay connection (and with an offer a WebRTC peer) before any
/// password.
pub const WEB_SESSIONS_PER_DEVICE_PER_MINUTE: u32 = 10;
const SWEEP_EVERY: u64 = 1024;

#[derive(Clone)]
struct Monotonic(SharedClock);

impl governor::clock::Clock for Monotonic {
    type Instant = Duration;

    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

type Buckets = RateLimiter<String, HashMapStateStore<String>, Monotonic, NoOpMiddleware<Duration>>;

pub(super) struct KeyedLimit {
    clock: Monotonic,
    buckets: Buckets,
    checks: AtomicU64,
}

impl KeyedLimit {
    pub(super) fn new(per_minute: u32, clock: SharedClock) -> Self {
        let clock = Monotonic(clock);
        let per_minute = NonZeroU32::new(per_minute).unwrap_or(NonZeroU32::MIN);
        Self {
            buckets: RateLimiter::new(
                Quota::per_minute(per_minute),
                HashMapStateStore::default(),
                clock.clone(),
            ),
            clock,
            checks: AtomicU64::new(0),
        }
    }

    pub(super) fn check(&self, key: &str) -> Result<(), Duration> {
        let now = governor::clock::Clock::now(&self.clock);
        let result = self
            .buckets
            .check_key(&key.to_owned())
            .map_err(|not_until| not_until.wait_time_from(now));
        if self.checks.fetch_add(1, Ordering::Relaxed) % SWEEP_EVERY == SWEEP_EVERY - 1 {
            self.buckets.retain_recent();
            self.buckets.shrink_to_fit();
        }
        result
    }
}
