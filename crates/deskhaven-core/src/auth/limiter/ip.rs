//! IPv6 is keyed by /64: one host usually owns a whole /64, and per-/128 buckets would be free
//! to multiply. Proxy-fallback requests (a trusted proxy's own address) share one bucket with
//! [`FALLBACK_FACTOR`] times the quota. Time is monotonic, so stepping the wall clock neither
//! refills nor drains buckets.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use arc_swap::ArcSwap;
use deskhaven_config::settings::AUTH_IP_RATE_LIMIT_PER_MINUTE;
use governor::middleware::NoOpMiddleware;
use governor::state::keyed::HashMapStateStore;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};

use super::ClientAddr;
use crate::clock::SharedClock;
use crate::settings::SettingsService;

pub const SWEEP_EVERY: u64 = 1024;
pub const FALLBACK_FACTOR: u32 = 10;
pub const FALLBACK_WARN_INTERVAL: Duration = Duration::from_secs(60);
const IPV6_PREFIX_MASK: u128 = !0 << 64;

pub fn ip_key(addr: IpAddr) -> IpAddr {
    match addr.to_canonical() {
        IpAddr::V6(v6) => IpAddr::V6(Ipv6Addr::from(u128::from(v6) & IPV6_PREFIX_MASK)),
        v4 => v4,
    }
}

#[derive(Clone)]
struct GovernorClock(SharedClock);

impl governor::clock::Clock for GovernorClock {
    type Instant = Duration;

    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

type Buckets =
    RateLimiter<IpAddr, HashMapStateStore<IpAddr>, GovernorClock, NoOpMiddleware<Duration>>;
type SharedBucket = RateLimiter<NotKeyed, InMemoryState, GovernorClock, NoOpMiddleware<Duration>>;

type BucketKey = Option<IpAddr>;

fn bucket_key(client: ClientAddr) -> BucketKey {
    (!client.is_proxy_fallback).then(|| ip_key(client.addr))
}

struct Configured {
    per_minute: u32,
    buckets: Buckets,
    fallback: SharedBucket,
    /// governor has no side-effect-free peek; this is what [`IpLimiter::over_limit`] reads.
    denied: Mutex<HashMap<BucketKey, Duration>>,
}

impl Configured {
    fn new(per_minute: u32, clock: &GovernorClock) -> Self {
        let quota = |n: u32| Quota::per_minute(NonZeroU32::new(n).unwrap_or(NonZeroU32::MIN));
        Self {
            per_minute,
            buckets: RateLimiter::new(
                quota(per_minute),
                HashMapStateStore::default(),
                clock.clone(),
            ),
            fallback: RateLimiter::direct_with_clock(
                quota(per_minute.saturating_mul(FALLBACK_FACTOR)),
                clock.clone(),
            ),
            denied: Mutex::default(),
        }
    }

    fn denied(&self) -> MutexGuard<'_, HashMap<BucketKey, Duration>> {
        // Map operations cannot leave it inconsistent; a panic elsewhere must not
        // disable the limiter.
        self.denied.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn deny(&self, key: BucketKey, now: Duration, wait: Duration) {
        self.denied().insert(key, now.saturating_add(wait));
    }

    fn sweep(&self, now: Duration) {
        sweep(&self.buckets);
        let mut denied = self.denied();
        denied.retain(|_, until| *until > now);
        denied.shrink_to_fit();
    }
}

enum QuotaSource {
    LoginSetting(SettingsService),
    Fixed(u32),
}

impl QuotaSource {
    fn per_minute(&self) -> u32 {
        match self {
            Self::LoginSetting(settings) => settings.snapshot().get(&AUTH_IP_RATE_LIMIT_PER_MINUTE),
            Self::Fixed(per_minute) => *per_minute,
        }
    }
}

pub struct IpLimiter {
    quota: QuotaSource,
    clock: GovernorClock,
    current: ArcSwap<Configured>,
    checks: AtomicU64,
    /// `elapsed` (ms) + 1 of the last fallback warning; 0 = never.
    fallback_warned: AtomicU64,
}

impl IpLimiter {
    pub fn new(settings: SettingsService, clock: SharedClock) -> Self {
        Self::with_quota(QuotaSource::LoginSetting(settings), clock)
    }

    pub fn fixed(per_minute: u32, clock: SharedClock) -> Self {
        Self::with_quota(QuotaSource::Fixed(per_minute), clock)
    }

    fn with_quota(quota: QuotaSource, clock: SharedClock) -> Self {
        let clock = GovernorClock(clock);
        let current = ArcSwap::from_pointee(Configured::new(quota.per_minute(), &clock));
        Self {
            quota,
            clock,
            current,
            checks: AtomicU64::new(0),
            fallback_warned: AtomicU64::new(0),
        }
    }

    pub fn check(&self, client: ClientAddr) -> Result<(), Duration> {
        self.take(client, false)
    }

    /// Only the callers that gate on `over_limit` use it, so logins drain the same quota but never
    /// trip the gate.
    pub fn check_recorded(&self, client: ClientAddr) -> Result<(), Duration> {
        self.take(client, true)
    }

    fn take(&self, client: ClientAddr, record_denial: bool) -> Result<(), Duration> {
        let current = self.configured();
        let now = governor::clock::Clock::now(&self.clock);
        let key = bucket_key(client);
        let result = match key {
            None => {
                self.warn_fallback(client.addr, now);
                current.fallback.check()
            }
            Some(addr) => current.buckets.check_key(&addr),
        }
        .map_err(|not_until| not_until.wait_time_from(now));
        if let (Err(wait), true) = (result, record_denial) {
            current.deny(key, now, wait);
        }
        if key.is_some()
            && self.checks.fetch_add(1, Ordering::Relaxed) % SWEEP_EVERY == SWEEP_EVERY - 1
        {
            current.sweep(now);
        }
        result
    }

    /// Plain [`check`](Self::check) denials do not count.
    pub fn over_limit(&self, client: ClientAddr) -> Option<Duration> {
        let current = self.configured();
        let now = governor::clock::Clock::now(&self.clock);
        let until = current.denied().get(&bucket_key(client)).copied()?;
        until.checked_sub(now).filter(|left| !left.is_zero())
    }

    pub fn sweep(&self) {
        let now = governor::clock::Clock::now(&self.clock);
        self.current.load().sweep(now);
    }

    pub fn tracked_clients(&self) -> usize {
        self.current.load().buckets.len()
    }

    pub fn denied_clients(&self) -> usize {
        self.current.load().denied().len()
    }

    fn warn_fallback(&self, addr: IpAddr, now: Duration) {
        let now_ms = u64::try_from(now.as_millis()).unwrap_or(u64::MAX - 1);
        let last = self.fallback_warned.load(Ordering::Relaxed);
        let interval = u64::try_from(FALLBACK_WARN_INTERVAL.as_millis()).unwrap_or(u64::MAX);
        let due = last == 0 || now_ms.saturating_add(1).saturating_sub(last) >= interval;
        let claimed = due
            && self
                .fallback_warned
                .compare_exchange(last, now_ms + 1, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok();
        if claimed {
            tracing::warn!(
                %addr,
                "rate-limited request resolved to a trusted proxy address (misconfigured proxy or a \
                 client inside trusted_proxies); all such requests share one rate-limit bucket"
            );
        }
    }

    fn configured(&self) -> Arc<Configured> {
        let want = self.quota.per_minute();
        let current = self.current.load_full();
        if current.per_minute == want {
            return current;
        }
        self.current.rcu(|cur| {
            if cur.per_minute == want {
                Arc::clone(cur)
            } else {
                Arc::new(Configured::new(want, &self.clock))
            }
        });
        tracing::info!(per_minute = want, "per-IP login rate limit reconfigured");
        self.current.load_full()
    }
}

fn sweep(buckets: &Buckets) {
    buckets.retain_recent();
    buckets.shrink_to_fit();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse()
            .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
    }

    #[test]
    fn ip_key_masks_ipv6_to_slash64_and_keeps_ipv4() {
        assert_eq!(ip_key(ip("2001:db8:1:2:3:4:5:6")), ip("2001:db8:1:2::"));
        assert_eq!(ip_key(ip("192.0.2.7")), ip("192.0.2.7"));
        assert_eq!(ip_key(ip("::ffff:192.0.2.7")), ip("192.0.2.7"));
        assert_ne!(ip_key(ip("2001:db8:1:2::1")), ip_key(ip("2001:db8:1:3::1")));
    }
}
