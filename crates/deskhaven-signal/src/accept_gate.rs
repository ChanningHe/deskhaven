//! Admission control of the main TCP port (deskhaven code, not vendored; M4-05).
//!
//! Each accepted main-port connection gets a KeyExchange offer at once (an X25519 key
//! generation and two Ed25519 signatures, about 100 µs), before the client has sent anything.
//! The gate decides at accept time, before any of that work, whether a connection may start
//! its handshake:
//!
//! - **per-IP accept budget**: a token bucket of `per_ip_burst` connections refilled at
//!   `per_ip_per_second`, bounding the CPU one source can spend on offers;
//! - **per-IP pending handshakes**: at most `per_ip_pending` connections of one source that have
//!   not sent their first frame;
//! - **global pending handshakes**: at most `pending` such connections in all, bounding memory
//!   and tasks under a flood of silent connections from many sources.
//!
//! IPv6 sources are keyed by their /64, which a single host usually controls in full. A
//! refused connection is simply closed. A [`Pending`] slot is released when it is dropped:
//! the connection sent its first frame or ended.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use hbb_common::log;

use crate::log_limit::LogLimit;

/// Refusals are logged at most once a minute, with their count.
static REFUSED_LOG: LogLimit = LogLimit::new(Duration::from_secs(60));

/// Most source keys tracked at once. Beyond it, unknown sources skip the per-IP limits (only
/// the global cap applies) rather than being refused, so a flood from many addresses cannot
/// lock new legitimate sources out. Idle keys are pruned on the server's timer.
const MAX_TRACKED: usize = 100_000;

/// Limits of [`AcceptGate`]; see the module docs. A zero disables nothing: it refuses
/// everything that limit covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AcceptLimits {
    /// Connections one source may open at once before its bucket is empty.
    pub per_ip_burst: u32,
    /// Bucket refill per second, per source.
    pub per_ip_per_second: u32,
    /// Connections of one source that have not sent their first frame.
    pub per_ip_pending: u32,
    /// Connections in all that have not sent their first frame.
    pub pending: u32,
}

impl Default for AcceptLimits {
    /// Sized so the RustDesk clients behind one NAT address (TCP-mode devices reconnecting
    /// after a server restart, controllers of a support desk) are not throttled noticeably:
    /// connections normally send their first frame within a round trip of the offer, so
    /// pending counts stay far below the caps, and one source refilling at 64/s spends well
    /// under 1% of a core on offers.
    fn default() -> Self {
        Self {
            per_ip_burst: 256,
            per_ip_per_second: 64,
            per_ip_pending: 64,
            pending: 2048,
        }
    }
}

/// Why a connection was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    /// The source's accept budget is used up.
    Rate,
    /// The source has `per_ip_pending` connections pending.
    SourcePending,
    /// The server has `pending` connections pending.
    Pending,
}

/// Shared admission state; cheap to clone.
#[derive(Clone)]
pub(crate) struct AcceptGate {
    limits: AcceptLimits,
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    pending: u32,
    sources: HashMap<IpAddr, Source>,
}

struct Source {
    tokens: f64,
    refilled: Instant,
    pending: u32,
}

/// A pending handshake slot; dropping it releases it.
pub(crate) struct Pending {
    state: Arc<Mutex<State>>,
    /// `None` when the source was not tracked (table full).
    key: Option<IpAddr>,
}

impl AcceptGate {
    pub(crate) fn new(limits: AcceptLimits) -> Self {
        Self {
            limits,
            state: Arc::default(),
        }
    }

    /// [`AcceptGate::admit`] now; a refusal is logged (at most once a minute, with a count).
    pub(crate) fn admit_logged(&self, ip: IpAddr) -> Option<Pending> {
        let now = Instant::now();
        let refused = match self.admit(ip, now) {
            Ok(pending) => return Some(pending),
            Err(refused) => refused,
        };
        if let Some(count) = REFUSED_LOG.hit(now) {
            log::warn!("refused {count} main-port connection(s); latest from {ip}: {refused:?}");
        }
        None
    }

    /// Admits a connection from `ip` at `now`, or says why not.
    pub(crate) fn admit(&self, ip: IpAddr, now: Instant) -> Result<Pending, Refused> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.pending >= self.limits.pending {
            return Err(Refused::Pending);
        }
        let key = source_key(ip);
        let tracked = state.sources.contains_key(&key) || state.sources.len() < MAX_TRACKED;
        if tracked {
            let source = state.sources.entry(key).or_insert_with(|| Source {
                tokens: f64::from(self.limits.per_ip_burst),
                refilled: now,
                pending: 0,
            });
            self.take(source, now)?;
        }
        state.pending += 1;
        Ok(Pending {
            state: Arc::clone(&self.state),
            key: tracked.then_some(key),
        })
    }

    /// Takes one token and one pending slot from `source`, or neither.
    fn take(&self, source: &mut Source, now: Instant) -> Result<(), Refused> {
        self.refill(source, now);
        if source.pending >= self.limits.per_ip_pending {
            return Err(Refused::SourcePending);
        }
        if source.tokens < 1.0 {
            return Err(Refused::Rate);
        }
        source.tokens -= 1.0;
        source.pending += 1;
        Ok(())
    }

    fn refill(&self, source: &mut Source, now: Instant) {
        let elapsed = now.saturating_duration_since(source.refilled).as_secs_f64();
        let rate = f64::from(self.limits.per_ip_per_second);
        source.tokens = (source.tokens + elapsed * rate).min(f64::from(self.limits.per_ip_burst));
        source.refilled = now;
    }

    /// Forgets sources with nothing pending and a full bucket: they are indistinguishable from
    /// sources never seen.
    pub(crate) fn prune(&self, now: Instant) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let burst = f64::from(self.limits.per_ip_burst);
        state.sources.retain(|_, source| {
            self.refill(source, now);
            source.pending > 0 || source.tokens < burst
        });
    }

    #[cfg(test)]
    fn tracked(&self) -> usize {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.sources.len()
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.pending = state.pending.saturating_sub(1);
        if let Some(source) = self.key.and_then(|key| state.sources.get_mut(&key)) {
            source.pending = source.pending.saturating_sub(1);
        }
    }
}

/// The address itself for IPv4 (IPv4-mapped IPv6 included), its /64 for IPv6.
pub(crate) fn source_key(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(v6) => {
            let net = u128::from(v6) & !u128::from(u64::MAX);
            IpAddr::V6(Ipv6Addr::from(net))
        }
        v4 => v4,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use std::net::Ipv4Addr;

    use super::*;

    const LIMITS: AcceptLimits = AcceptLimits {
        per_ip_burst: 4,
        per_ip_per_second: 2,
        per_ip_pending: 3,
        pending: 5,
    };

    fn v4(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, last))
    }

    fn v6(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn a_source_gets_its_burst_then_the_refill_rate() {
        let gate = AcceptGate::new(LIMITS);
        let t0 = Instant::now();
        for _ in 0..4 {
            drop(gate.admit(v4(1), t0).unwrap());
        }

        assert_eq!(gate.admit(v4(1), t0).err(), Some(Refused::Rate));
        assert!(gate.admit(v4(2), t0).is_ok());
        let later = t0 + Duration::from_millis(500);
        drop(gate.admit(v4(1), later).unwrap());
        assert_eq!(gate.admit(v4(1), later).err(), Some(Refused::Rate));
    }

    #[test]
    fn a_source_has_at_most_per_ip_pending_slots_until_one_is_released() {
        let gate = AcceptGate::new(LIMITS);
        let t0 = Instant::now();
        let held: Vec<_> = (0..3).map(|_| gate.admit(v4(1), t0).unwrap()).collect();

        assert_eq!(gate.admit(v4(1), t0).err(), Some(Refused::SourcePending));
        drop(held);
        assert!(gate.admit(v4(1), t0).is_ok());
    }

    #[test]
    fn the_global_cap_counts_every_source() {
        let gate = AcceptGate::new(LIMITS);
        let t0 = Instant::now();
        let held: Vec<_> = (1..=5).map(|n| gate.admit(v4(n), t0).unwrap()).collect();

        assert_eq!(gate.admit(v4(9), t0).err(), Some(Refused::Pending));
        drop(held);
        assert!(gate.admit(v4(9), t0).is_ok());
    }

    #[test]
    fn a_refusal_takes_neither_a_token_nor_a_slot() {
        let gate = AcceptGate::new(AcceptLimits {
            per_ip_pending: 1,
            ..LIMITS
        });
        let t0 = Instant::now();
        let held = gate.admit(v4(1), t0).unwrap();
        for _ in 0..10 {
            assert_eq!(gate.admit(v4(1), t0).err(), Some(Refused::SourcePending));
        }
        drop(held);

        let left = (0..10).filter(|_| gate.admit(v4(1), t0).is_ok()).count();

        assert_eq!(left, 3);
    }

    #[test]
    fn ipv6_sources_share_their_slash_64_and_mapped_ipv4_is_ipv4() {
        let gate = AcceptGate::new(LIMITS);
        let t0 = Instant::now();
        let _held: Vec<_> = ["2001:db8::1", "2001:db8::2", "2001:db8::ffff:1"]
            .into_iter()
            .map(|ip| gate.admit(v6(ip), t0).unwrap())
            .collect();

        assert_eq!(
            gate.admit(v6("2001:db8:0:0:1::1"), t0).err(),
            Some(Refused::SourcePending)
        );
        assert!(gate.admit(v6("2001:db8:0:1::1"), t0).is_ok());
        assert_eq!(source_key(v6("::ffff:192.0.2.1")), v4(1));
    }

    #[test]
    fn prune_forgets_only_idle_sources_with_a_full_bucket() {
        let gate = AcceptGate::new(LIMITS);
        let t0 = Instant::now();
        let held = gate.admit(v4(1), t0).unwrap();
        drop(gate.admit(v4(2), t0).unwrap());
        drop(gate.admit(v4(3), t0).unwrap());

        gate.prune(t0);
        assert_eq!(gate.tracked(), 3);
        gate.prune(t0 + Duration::from_secs(1));
        assert_eq!(gate.tracked(), 1);
        drop(held);
        gate.prune(t0 + Duration::from_secs(1));
        assert_eq!(gate.tracked(), 0);
    }
}
