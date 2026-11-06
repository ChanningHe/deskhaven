//! Per-source RegisterPk limits (deskhaven code; was upstream's `IP_BLOCKER` map and
//! `check_ip_blocker`, M4-05).
//!
//! Upstream semantics per source: at most 31 RegisterPk within a 60 s window (the counter
//! resets once 60 s passed since the last accepted one), and at most 301 distinct ids per day
//! (known ids still pass once the set is full). deskhaven changes how sources are kept, not
//! those limits:
//!
//! - a source is the IPv4 address (IPv4-mapped IPv6 included) or the IPv6 /64, so one host
//!   cannot spread over its 2^64 addresses;
//! - ids are kept as 64-bit keyed hashes (8 bytes each instead of a copy of an id of up to
//!   253 bytes), which bounds an entry to a few KiB;
//! - the table is capped at `max_entries` sources; when it is full, the least recently seen
//!   tenth is evicted (one linear pass, amortised over the following inserts) instead of
//!   refusing every new source, so a flood of addresses cannot lock new devices out. An
//!   evicted source merely starts over with fresh counters.

use std::collections::HashMap;
use std::hash::{BuildHasher, RandomState};
use std::net::IpAddr;
use std::time::Instant;

use crate::accept_gate::source_key;
use crate::peer::{DAY_SECONDS, IP_BLOCK_DUR};

/// RegisterPk accepted per source and window (the counter refuses once it exceeds this).
const MAX_REGISTERS: u32 = 30;
/// Distinct ids per source and day (new ids are refused once the set exceeds this).
const MAX_IDS: usize = 300;
/// Sources tracked at once (one per source seen in the last day). Worst case about 125 MB:
/// 301 id hashes of 8 bytes per source plus the map overhead.
pub(crate) const MAX_ENTRIES: usize = 50_000;
/// Share of the table evicted when it is full.
const EVICT_DIVISOR: usize = 10;

struct Entry {
    registers: u32,
    registered_at: Instant,
    /// Sorted id hashes.
    ids: Vec<u64>,
    ids_at: Instant,
}

/// The per-source RegisterPk accounting.
pub(crate) struct IpBlocker {
    entries: HashMap<IpAddr, Entry>,
    max_entries: usize,
    id_hasher: RandomState,
}

impl Default for IpBlocker {
    fn default() -> Self {
        Self::new(MAX_ENTRIES)
    }
}

impl IpBlocker {
    pub(crate) fn new(max_entries: usize) -> Self {
        Self {
            entries: HashMap::new(),
            max_entries: max_entries.max(1),
            id_hasher: RandomState::new(),
        }
    }

    /// Whether a RegisterPk for `id` from `ip` at `now` may proceed (upstream
    /// `check_ip_blocker`).
    pub(crate) fn check(&mut self, ip: IpAddr, id: &str, now: Instant) -> bool {
        let key = source_key(ip);
        if !self.entries.contains_key(&key) {
            if self.entries.len() >= self.max_entries {
                self.evict_oldest();
            }
            let fresh = Entry {
                registers: 0,
                registered_at: now,
                ids: Vec::new(),
                ids_at: now,
            };
            self.entries.insert(key, fresh);
            return true;
        }
        let hash = self.id_hasher.hash_one(id);
        self.entries
            .get_mut(&key)
            .is_none_or(|entry| entry.check(hash, now))
    }

    /// Forgets sources whose register window and id day have both passed (upstream's `ib`
    /// predicate).
    pub(crate) fn prune(&mut self, now: Instant) {
        self.entries.retain(|_, entry| {
            !expired(entry.registered_at, now, IP_BLOCK_DUR)
                || !expired(entry.ids_at, now, DAY_SECONDS)
        });
    }

    /// Drops the least recently seen tenth of the entries.
    fn evict_oldest(&mut self) {
        let mut seen: Vec<Instant> = self.entries.values().map(Entry::last_seen).collect();
        let cut = (seen.len() / EVICT_DIVISOR).max(1).min(seen.len()) - 1;
        let (_, threshold, _) = seen.select_nth_unstable(cut);
        let threshold = *threshold;
        self.entries
            .retain(|_, entry| entry.last_seen() > threshold);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    fn ids_of(&self, ip: IpAddr) -> usize {
        self.entries.get(&source_key(ip)).map_or(0, |e| e.ids.len())
    }
}

impl Entry {
    fn check(&mut self, hash: u64, now: Instant) -> bool {
        if expired(self.registered_at, now, IP_BLOCK_DUR) {
            self.registers = 0;
        } else if self.registers > MAX_REGISTERS {
            return false;
        }
        self.registers += 1;
        self.registered_at = now;

        if expired(self.ids_at, now, DAY_SECONDS) {
            self.ids.clear();
        } else if self.ids.len() > MAX_IDS {
            return self.ids.binary_search(&hash).is_ok();
        }
        if let Err(at) = self.ids.binary_search(&hash) {
            self.ids.reserve_exact(1);
            self.ids.insert(at, hash);
        }
        self.ids_at = now;
        true
    }

    fn last_seen(&self) -> Instant {
        self.registered_at.max(self.ids_at)
    }
}

/// Whether more than `secs` whole seconds passed from `at` to `now` (upstream compared
/// `elapsed().as_secs() > secs`).
fn expired(at: Instant, now: Instant, secs: u64) -> bool {
    now.saturating_duration_since(at).as_secs() > secs
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use std::net::Ipv4Addr;
    use std::time::Duration;

    use super::*;

    fn v4(n: u32) -> IpAddr {
        IpAddr::V4(Ipv4Addr::from(0xC000_0200 + n))
    }

    fn v6(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn a_source_gets_31_registers_per_window_like_upstream() {
        let mut blocker = IpBlocker::new(8);
        let t0 = Instant::now();
        let ok = (0..40).filter(|_| blocker.check(v4(1), "id-1", t0)).count();
        assert_eq!(ok, 32, "the insert plus 31 counted registers");
        let later = t0 + Duration::from_secs(IP_BLOCK_DUR + 1);
        assert!(blocker.check(v4(1), "id-1", later), "a new window");
    }

    #[test]
    fn a_source_gets_301_distinct_ids_per_day_and_known_ids_still_pass() {
        let mut blocker = IpBlocker::new(8);
        let mut now = Instant::now();
        assert!(blocker.check(v4(1), "first", now));
        let mut accepted = 0;
        for n in 0..1000 {
            // One register a minute, so the register window never refuses.
            now += Duration::from_secs(IP_BLOCK_DUR + 1);
            accepted += usize::from(blocker.check(v4(1), &format!("id-{n}"), now));
        }
        assert_eq!(accepted, MAX_IDS + 1);
        assert_eq!(
            blocker.ids_of(v4(1)),
            MAX_IDS + 1,
            "bounded: 8 bytes per id"
        );
        now += Duration::from_secs(IP_BLOCK_DUR + 1);
        assert!(blocker.check(v4(1), "id-0", now), "known id");
        assert!(!blocker.check(v4(1), "id-new", now), "new id");
    }

    #[test]
    fn a_flood_of_sources_cannot_lock_a_fresh_source_out() {
        let mut blocker = IpBlocker::new(10);
        let t0 = Instant::now();
        for n in 0..1000 {
            let at = t0 + Duration::from_millis(u64::from(n));
            assert!(blocker.check(v4(n), "flood-id", at), "source {n}");
            assert!(blocker.len() <= 10);
        }

        let later = t0 + Duration::from_secs(2);
        assert!(blocker.check(v4(5000), "fresh-id", later));
        assert!(
            blocker.check(v4(5000), "fresh-id", later),
            "and it is tracked"
        );
        assert!(blocker.len() <= 10);
    }

    #[test]
    fn eviction_drops_the_least_recently_seen_sources() {
        let mut blocker = IpBlocker::new(10);
        let t0 = Instant::now();
        for n in 0..10 {
            blocker.check(v4(n), "id", t0 + Duration::from_secs(u64::from(n)));
        }
        // v4(0) is the oldest; refresh it so v4(1) becomes the oldest.
        blocker.check(v4(0), "id", t0 + Duration::from_secs(20));

        blocker.check(v4(99), "id", t0 + Duration::from_secs(21));

        assert_eq!(blocker.len(), 10);
        assert!(blocker.entries.contains_key(&v4(0)));
        assert!(!blocker.entries.contains_key(&v4(1)));
    }

    #[test]
    fn ipv6_sources_share_their_slash_64_and_mapped_ipv4_is_ipv4() {
        let mut blocker = IpBlocker::new(8);
        let t0 = Instant::now();
        blocker.check(v6("2001:db8::1"), "a-id-1", t0);
        blocker.check(v6("2001:db8::ffff:2"), "a-id-2", t0);
        blocker.check(v6("::ffff:192.0.2.1"), "b-id-1", t0);
        blocker.check(v4(1), "b-id-2", t0);

        assert_eq!(blocker.len(), 2);
        assert_eq!(blocker.ids_of(v6("2001:db8::7")), 1);
    }

    #[test]
    fn prune_keeps_a_source_while_either_window_is_live() {
        let mut blocker = IpBlocker::new(8);
        let t0 = Instant::now();
        let day = Duration::from_secs(DAY_SECONDS);
        blocker.check(v4(1), "id", t0);
        blocker.check(v4(2), "id", t0);
        blocker.check(v4(2), "id", t0 + day);

        blocker.prune(t0 + day);
        assert_eq!(blocker.len(), 2, "the id day is live for both");
        blocker.prune(t0 + day + Duration::from_secs(2));
        assert_eq!(blocker.len(), 1, "v4(1): both windows passed");
        blocker.prune(t0 + day * 2 + Duration::from_secs(2));
        assert_eq!(blocker.len(), 0);
    }

    /// Re-review LOW-3: 50 000 sources of at most 301 id hashes each, about 125 MB at worst.
    #[test]
    fn the_default_table_tracks_at_most_50_000_sources() {
        let mut blocker = IpBlocker::default();
        let t0 = Instant::now();
        for n in 0..50_001u32 {
            blocker.check(IpAddr::V4(Ipv4Addr::from(0x0A00_0000 + n)), "id", t0);
        }

        assert!(blocker.len() <= 50_000, "{}", blocker.len());
    }
}
