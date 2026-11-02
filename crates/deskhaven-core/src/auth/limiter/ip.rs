//! Per-client token bucket for login-type endpoints (governor keyed GCRA, in memory).
//!
//! - Key: the IPv4 address (/32), or the IPv6 /64 the address is in (one host usually owns a
//!   whole /64; per-/128 buckets would be free to multiply). IPv4-mapped IPv6 counts as IPv4.
//! - Quota: `auth.ip_rate_limit.per_minute` requests, refilled evenly over a minute (burst =
//!   the per-minute value). Read from the settings snapshot on every check; when it changed,
//!   the limiter is rebuilt and every bucket starts full again.
//! - A [`ClientAddr`] with `is_proxy_fallback` is a trusted proxy's own address, shared by
//!   every client behind it: it is never limited (logged at debug). Account lockout still
//!   applies to such requests.
//! - Memory: buckets that refilled completely are indistinguishable from absent ones and are
//!   dropped every [`SWEEP_EVERY`] checks and on [`IpLimiter::sweep`]. The map is therefore
//!   bounded by the number of distinct clients seen within one minute.
//! - State is per process and lost on restart (the persistent part is the account lockout).

use std::net::{IpAddr, Ipv6Addr};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use arc_swap::ArcSwap;
use deskhaven_config::settings::AUTH_IP_RATE_LIMIT_PER_MINUTE;
use governor::middleware::NoOpMiddleware;
use governor::state::keyed::HashMapStateStore;
use governor::{Quota, RateLimiter};

use super::ClientAddr;
use crate::clock::SharedClock;
use crate::settings::SettingsService;

/// Checks between two sweeps of refilled buckets.
pub const SWEEP_EVERY: u64 = 1024;
/// Keeps the upper 64 bits of an IPv6 address.
const IPV6_PREFIX_MASK: u128 = !0 << 64;

/// The bucket key of `addr`: the canonical IPv4 address, or its IPv6 /64 network.
pub fn ip_key(addr: IpAddr) -> IpAddr {
    match addr.to_canonical() {
        IpAddr::V6(v6) => IpAddr::V6(Ipv6Addr::from(u128::from(v6) & IPV6_PREFIX_MASK)),
        v4 => v4,
    }
}

/// governor's view of the injected [`Clock`](crate::clock::Clock): time since the epoch.
#[derive(Clone)]
struct GovernorClock(SharedClock);

impl governor::clock::Clock for GovernorClock {
    type Instant = Duration;

    fn now(&self) -> Duration {
        Duration::from_millis(u64::try_from(self.0.now_ms()).unwrap_or(0))
    }
}

type Buckets =
    RateLimiter<IpAddr, HashMapStateStore<IpAddr>, GovernorClock, NoOpMiddleware<Duration>>;

/// One limiter for one quota.
struct Configured {
    per_minute: u32,
    buckets: Buckets,
}

impl Configured {
    fn new(per_minute: u32, clock: &GovernorClock) -> Self {
        let burst = NonZeroU32::new(per_minute).unwrap_or(NonZeroU32::MIN);
        let buckets = RateLimiter::new(
            Quota::per_minute(burst),
            HashMapStateStore::default(),
            clock.clone(),
        );
        Self {
            per_minute,
            buckets,
        }
    }
}

/// Token bucket per client network. Cheap to share (`&self` everywhere).
pub struct IpLimiter {
    settings: SettingsService,
    clock: GovernorClock,
    current: ArcSwap<Configured>,
    checks: AtomicU64,
}

impl IpLimiter {
    pub fn new(settings: SettingsService, clock: SharedClock) -> Self {
        let clock = GovernorClock(clock);
        let per_minute = settings.snapshot().get(&AUTH_IP_RATE_LIMIT_PER_MINUTE);
        let current = ArcSwap::from_pointee(Configured::new(per_minute, &clock));
        Self {
            settings,
            clock,
            current,
            checks: AtomicU64::new(0),
        }
    }

    /// Takes one token for `client`, or returns how long until one is available.
    pub fn check(&self, client: ClientAddr) -> Result<(), Duration> {
        if client.is_proxy_fallback {
            tracing::debug!(addr = %client.addr, "trusted proxy address; per-IP login limit skipped");
            return Ok(());
        }
        let current = self.configured();
        let result = current
            .buckets
            .check_key(&ip_key(client.addr))
            .map_err(|not_until| {
                not_until.wait_time_from(governor::clock::Clock::now(&self.clock))
            });
        if self.checks.fetch_add(1, Ordering::Relaxed) % SWEEP_EVERY == SWEEP_EVERY - 1 {
            sweep(&current.buckets);
        }
        result
    }

    /// Drops buckets that refilled completely.
    pub fn sweep(&self) {
        sweep(&self.current.load().buckets);
    }

    /// Number of client networks currently tracked.
    pub fn tracked_clients(&self) -> usize {
        self.current.load().buckets.len()
    }

    /// The limiter for the current setting, rebuilt if the setting changed.
    fn configured(&self) -> Arc<Configured> {
        let want = self.settings.snapshot().get(&AUTH_IP_RATE_LIMIT_PER_MINUTE);
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
