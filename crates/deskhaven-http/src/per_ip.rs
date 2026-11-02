//! Concurrent connection cap per client, enforced at accept (review H-a).
//!
//! A "client" is an IPv4 address (/32) or an IPv6 prefix (`/per_ip_ipv6_prefix`,
//! default /64): one host usually owns a whole /64, so keying by /128 would let it
//! open thousands of "different" clients. The cap per key is adaptive
//! ([`per_key_limit`]) so a handful of keys can never fill the global cap.
//!
//! Trusted proxies are exempt: every client behind them shares their address.
//! A broad `trusted_proxies` range therefore exempts every host inside it; keep
//! trusted ranges as narrow as possible.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ipnet::Ipv6Net;

use crate::real_ip::TrustedProxies;

/// At most one warning per interval; individual rejections are logged at debug.
const WARN_INTERVAL: Duration = Duration::from_secs(60);
/// The adaptive cap never goes below this many connections per key.
const MIN_PER_KEY: usize = 16;
/// ... and never above 1/64 of the listener's connection cap.
const GLOBAL_SHARE_DIVISOR: usize = 64;

/// `min(configured, max(16, max_connections / 64))`, at least 1.
pub(crate) fn per_key_limit(configured: usize, max_connections: usize) -> usize {
    let share = (max_connections / GLOBAL_SHARE_DIVISOR).max(MIN_PER_KEY);
    configured.min(share).max(1)
}

/// The budget key for `addr`: the IPv4 address, or its IPv6 `/prefix` network.
pub(crate) fn limit_key(addr: IpAddr, ipv6_prefix: u8) -> IpAddr {
    match addr.to_canonical() {
        IpAddr::V6(v6) => {
            Ipv6Net::new(v6, ipv6_prefix).map_or(IpAddr::V6(v6), |net| IpAddr::V6(net.network()))
        }
        v4 => v4,
    }
}

type Counts = Arc<Mutex<HashMap<IpAddr, usize>>>;

pub(crate) struct PerIpLimiter {
    limit: usize,
    ipv6_prefix: u8,
    trusted: TrustedProxies,
    counts: Counts,
    rejected_since_warn: u64,
    last_warn: Option<Instant>,
}

/// Held by a connection task; releases the key's slot when dropped
/// (normal close, error, abort or panic).
pub(crate) struct Slot(Option<(Counts, IpAddr)>);

impl Drop for Slot {
    fn drop(&mut self) {
        let Some((counts, key)) = self.0.take() else {
            return;
        };
        let mut counts = counts.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(n) = counts.get_mut(&key) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                counts.remove(&key);
            }
        }
    }
}

impl PerIpLimiter {
    /// `limit` is the effective per-key cap (see [`per_key_limit`]).
    pub(crate) fn new(limit: usize, ipv6_prefix: u8, trusted: TrustedProxies) -> Self {
        Self {
            limit: limit.max(1),
            ipv6_prefix,
            trusted,
            counts: Counts::default(),
            rejected_since_warn: 0,
            last_warn: None,
        }
    }

    /// A slot for a new connection from `peer`, or `None` if its key is over the cap.
    pub(crate) fn try_acquire(&mut self, peer: IpAddr) -> Option<Slot> {
        if self.trusted.is_trusted(peer) {
            return Some(Slot(None));
        }
        let key = limit_key(peer, self.ipv6_prefix);
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        let n = counts.entry(key).or_insert(0);
        if *n >= self.limit {
            drop(counts);
            self.note_rejection(key);
            return None;
        }
        *n += 1;
        Some(Slot(Some((Arc::clone(&self.counts), key))))
    }

    fn note_rejection(&mut self, key: IpAddr) {
        tracing::debug!(client = %key, limit = self.limit, "per-IP connection limit reached");
        self.rejected_since_warn += 1;
        let now = Instant::now();
        if self
            .last_warn
            .is_some_and(|at| now.duration_since(at) < WARN_INTERVAL)
        {
            return;
        }
        tracing::warn!(
            rejected = self.rejected_since_warn,
            limit = self.limit,
            "connections rejected by the per-IP limit"
        );
        self.rejected_since_warn = 0;
        self.last_warn = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn limiter(limit: usize) -> PerIpLimiter {
        PerIpLimiter::new(limit, 64, TrustedProxies::default())
    }

    #[test]
    fn caps_each_address_independently() {
        let mut limiter = limiter(2);
        let a1 = limiter.try_acquire(ip("192.0.2.1"));
        let a2 = limiter.try_acquire(ip("192.0.2.1"));
        assert!(a1.is_some() && a2.is_some());
        assert!(limiter.try_acquire(ip("192.0.2.1")).is_none());
        assert!(limiter.try_acquire(ip("192.0.2.2")).is_some());
    }

    #[test]
    fn dropping_a_slot_frees_it_and_forgets_idle_keys() {
        let mut limiter = limiter(1);
        let slot = limiter.try_acquire(ip("192.0.2.1"));
        assert!(limiter.try_acquire(ip("192.0.2.1")).is_none());
        drop(slot);
        assert!(limiter.counts.lock().unwrap().is_empty());
        assert!(limiter.try_acquire(ip("192.0.2.1")).is_some());
    }

    #[test]
    fn mapped_and_plain_ipv4_share_a_budget() {
        let mut limiter = limiter(1);
        let _slot = limiter.try_acquire(ip("192.0.2.1"));
        assert!(limiter.try_acquire(ip("::ffff:192.0.2.1")).is_none());
    }

    /// Review round 3 HIGH: rotating addresses inside one /64 gains nothing.
    #[test]
    fn ipv6_addresses_in_one_slash64_share_a_budget() {
        let mut limiter = limiter(2);
        let _a = limiter.try_acquire(ip("2001:db8:1:1::1"));
        let _b = limiter.try_acquire(ip("2001:db8:1:1:ffff:ffff:ffff:ffff"));
        assert!(limiter.try_acquire(ip("2001:db8:1:1::abcd")).is_none());
    }

    #[test]
    fn different_slash64s_have_separate_budgets() {
        let mut limiter = limiter(1);
        let _a = limiter.try_acquire(ip("2001:db8:1:1::1"));
        assert!(limiter.try_acquire(ip("2001:db8:1:2::1")).is_some());
    }

    #[test]
    fn ipv6_prefix_is_configurable() {
        let mut per_host = PerIpLimiter::new(1, 128, TrustedProxies::default());
        let _a = per_host.try_acquire(ip("2001:db8:1:1::1"));
        assert!(per_host.try_acquire(ip("2001:db8:1:1::2")).is_some());

        let mut per_48 = PerIpLimiter::new(1, 48, TrustedProxies::default());
        let _b = per_48.try_acquire(ip("2001:db8:1:1::1"));
        assert!(per_48.try_acquire(ip("2001:db8:1:ffff::1")).is_none());
    }

    #[test]
    fn limit_key_masks_ipv6_and_keeps_ipv4() {
        assert_eq!(
            limit_key(ip("2001:db8:1:1:2:3:4:5"), 64),
            ip("2001:db8:1:1::")
        );
        assert_eq!(limit_key(ip("192.0.2.7"), 64), ip("192.0.2.7"));
        assert_eq!(limit_key(ip("::ffff:192.0.2.7"), 64), ip("192.0.2.7"));
    }

    #[test]
    fn adaptive_cap_math() {
        // Defaults: 256 configured, 10 000 connections -> 10 000 / 64 = 156.
        assert_eq!(per_key_limit(256, 10_000), 156);
        // Small listeners keep a floor of 16 ...
        assert_eq!(per_key_limit(256, 100), 16);
        // ... unless the operator configured less.
        assert_eq!(per_key_limit(8, 100_000), 8);
        // Large listeners are bounded by the configured value.
        assert_eq!(per_key_limit(256, 1_000_000), 256);
        assert_eq!(per_key_limit(0, 0), 1);
    }

    #[test]
    fn a_few_keys_cannot_fill_the_global_cap() {
        let max_connections = 10_000;
        let per_key = per_key_limit(usize::MAX, max_connections);
        assert!(
            per_key * 40 < max_connections,
            "40 keys x {per_key} fill the cap"
        );
    }

    #[test]
    fn trusted_proxies_are_exempt() {
        let trusted = TrustedProxies::new(["10.0.0.0/8".parse().unwrap()]);
        let mut limiter = PerIpLimiter::new(1, 64, trusted);
        let slots: Vec<_> = (0..5)
            .map(|_| limiter.try_acquire(ip("10.0.0.1")))
            .collect();
        assert!(slots.iter().all(Option::is_some));
    }
}
