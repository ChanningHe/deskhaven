//! Concurrent connection cap per client address, enforced at accept (review H-a).
//! Trusted proxies are exempt: every client behind them shares their address.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::real_ip::TrustedProxies;

/// At most one warning per interval; individual rejections are logged at debug.
const WARN_INTERVAL: Duration = Duration::from_secs(60);

type Counts = Arc<Mutex<HashMap<IpAddr, usize>>>;

pub(crate) struct PerIpLimiter {
    limit: usize,
    trusted: TrustedProxies,
    counts: Counts,
    rejected_since_warn: u64,
    last_warn: Option<Instant>,
}

/// Held by a connection task; releases the address's slot when dropped
/// (normal close, error, abort or panic).
pub(crate) struct Slot(Option<(Counts, IpAddr)>);

impl Drop for Slot {
    fn drop(&mut self) {
        let Some((counts, addr)) = self.0.take() else {
            return;
        };
        let mut counts = counts.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(n) = counts.get_mut(&addr) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                counts.remove(&addr);
            }
        }
    }
}

impl PerIpLimiter {
    pub(crate) fn new(limit: usize, trusted: TrustedProxies) -> Self {
        Self {
            limit: limit.max(1),
            trusted,
            counts: Counts::default(),
            rejected_since_warn: 0,
            last_warn: None,
        }
    }

    /// A slot for a new connection from `peer`, or `None` if it is over its cap.
    pub(crate) fn try_acquire(&mut self, peer: IpAddr) -> Option<Slot> {
        let addr = peer.to_canonical();
        if self.trusted.is_trusted(addr) {
            return Some(Slot(None));
        }
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        let n = counts.entry(addr).or_insert(0);
        if *n >= self.limit {
            drop(counts);
            self.note_rejection(addr);
            return None;
        }
        *n += 1;
        Some(Slot(Some((Arc::clone(&self.counts), addr))))
    }

    fn note_rejection(&mut self, addr: IpAddr) {
        tracing::debug!(peer = %addr, limit = self.limit, "per-IP connection limit reached");
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

    #[test]
    fn caps_each_address_independently() {
        let mut limiter = PerIpLimiter::new(2, TrustedProxies::default());
        let a1 = limiter.try_acquire(ip("192.0.2.1"));
        let a2 = limiter.try_acquire(ip("192.0.2.1"));
        assert!(a1.is_some() && a2.is_some());
        assert!(limiter.try_acquire(ip("192.0.2.1")).is_none());
        assert!(limiter.try_acquire(ip("192.0.2.2")).is_some());
    }

    #[test]
    fn dropping_a_slot_frees_it_and_forgets_idle_addresses() {
        let mut limiter = PerIpLimiter::new(1, TrustedProxies::default());
        let slot = limiter.try_acquire(ip("192.0.2.1"));
        assert!(limiter.try_acquire(ip("192.0.2.1")).is_none());
        drop(slot);
        assert!(limiter.counts.lock().unwrap().is_empty());
        assert!(limiter.try_acquire(ip("192.0.2.1")).is_some());
    }

    #[test]
    fn mapped_and_plain_ipv4_share_a_budget() {
        let mut limiter = PerIpLimiter::new(1, TrustedProxies::default());
        let _slot = limiter.try_acquire(ip("192.0.2.1"));
        assert!(limiter.try_acquire(ip("::ffff:192.0.2.1")).is_none());
    }

    #[test]
    fn trusted_proxies_are_exempt() {
        let trusted = TrustedProxies::new(["10.0.0.0/8".parse().unwrap()]);
        let mut limiter = PerIpLimiter::new(1, trusted);
        let slots: Vec<_> = (0..5)
            .map(|_| limiter.try_acquire(ip("10.0.0.1")))
            .collect();
        assert!(slots.iter().all(Option::is_some));
    }
}
