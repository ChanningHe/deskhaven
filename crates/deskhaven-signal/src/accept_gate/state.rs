//! The accept gate's bookkeeping: per-source and per-/48 counters, the pending sets of both
//! ports and the eviction order of every connection. All updates are O(log n) except the
//! batched source pruning when the source table is full.

use std::collections::{BTreeSet, HashMap};
use std::net::IpAddr;
use std::time::Instant;

use tokio_util::sync::CancellationToken;

use super::loc::{Loc, source_key};
use super::pending_set::PendingSet;
use super::{AcceptLimits, Refused};

/// Most sources tracked at once; more than the largest connection cap, so there are always
/// idle sources to forget when it is full.
const MAX_TRACKED: usize = 100_000;
/// Share of the idle sources forgotten when the table is full (amortises the scan).
const FORGET_DIVISOR: usize = 10;

/// What a connection is worth keeping; eviction takes the smallest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Class {
    /// Has not sent its first frame (main port) or finished its handshake (websocket).
    Pending,
    /// Established, no registered peer on it.
    Idle,
    /// A registered peer's channel.
    Peer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Port {
    Main,
    Ws,
}

struct Conn {
    class: Class,
    stamp: u64,
    port: Port,
    /// Pending-set group (the socket peer's source, trusted proxies included).
    group: IpAddr,
    /// Counted as pending there (tracked sources only).
    pending_at: Option<Loc>,
    /// Counted as a connection there.
    conn_at: Option<Loc>,
    token: CancellationToken,
}

struct Source {
    tokens: f64,
    refilled: Instant,
    seen: Instant,
    pending: u32,
    connections: u32,
}

#[derive(Default)]
struct NetCount {
    pending: u32,
    connections: u32,
}

#[derive(Default)]
pub(crate) struct State {
    next: u64,
    conns: HashMap<u64, Conn>,
    /// (class, stamp, id) of every connection; the first is the least valuable.
    order: BTreeSet<(Class, u64, u64)>,
    main_pending: PendingSet,
    ws_pending: PendingSet,
    sources: HashMap<IpAddr, Source>,
    nets: HashMap<IpAddr, NetCount>,
}

impl State {
    fn next(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    pub(crate) fn len(&self) -> usize {
        self.conns.len()
    }

    pub(crate) fn pending_len(&self, port: Port) -> usize {
        match port {
            Port::Main => self.main_pending.len(),
            Port::Ws => self.ws_pending.len(),
        }
    }

    fn pending_set(&mut self, port: Port) -> &mut PendingSet {
        match port {
            Port::Main => &mut self.main_pending,
            Port::Ws => &mut self.ws_pending,
        }
    }

    /// Makes sure `ip`'s source is tracked (bucket refilled); `false` if the table is full of
    /// sources that all hold connections.
    pub(crate) fn track(&mut self, ip: IpAddr, limits: &AcceptLimits, now: Instant) -> bool {
        let key = source_key(ip);
        if !self.sources.contains_key(&key) && self.sources.len() >= MAX_TRACKED {
            self.forget_idle_sources();
            if self.sources.len() >= MAX_TRACKED {
                return false;
            }
        }
        let source = self.sources.entry(key).or_insert_with(|| Source {
            tokens: f64::from(limits.per_ip_burst),
            refilled: now,
            seen: now,
            pending: 0,
            connections: 0,
        });
        source.refill(limits, now);
        source.seen = now;
        true
    }

    /// Forgets the least recently seen tenth of the sources without connections.
    fn forget_idle_sources(&mut self) {
        let mut seen: Vec<Instant> = (self.sources.values())
            .filter(|s| s.pending == 0 && s.connections == 0)
            .map(|s| s.seen)
            .collect();
        if seen.is_empty() {
            return;
        }
        let cut = (seen.len() / FORGET_DIVISOR).max(1) - 1;
        let (_, threshold, _) = seen.select_nth_unstable(cut);
        let threshold = *threshold;
        (self.sources).retain(|_, s| s.pending > 0 || s.connections > 0 || s.seen > threshold);
    }

    pub(crate) fn check_connections(
        &self,
        ip: IpAddr,
        tracked: bool,
        limits: &AcceptLimits,
    ) -> Result<(), Refused> {
        if !tracked {
            return Ok(());
        }
        let loc = Loc::of(ip);
        let source = self.sources.get(&loc.key).map_or(0, |s| s.connections);
        if source >= limits.per_ip_connections {
            return Err(Refused::SourceConnections);
        }
        if self
            .net(loc)
            .is_some_and(|n| n.connections >= limits.per_net_connections)
        {
            return Err(Refused::NetConnections);
        }
        Ok(())
    }

    pub(crate) fn check_pending(
        &self,
        ip: IpAddr,
        tracked: bool,
        limits: &AcceptLimits,
        rate: bool,
    ) -> Result<(), Refused> {
        if !tracked {
            return Ok(());
        }
        let loc = Loc::of(ip);
        let Some(source) = self.sources.get(&loc.key) else {
            return Ok(());
        };
        if source.pending >= limits.per_ip_pending {
            return Err(Refused::SourcePending);
        }
        if self
            .net(loc)
            .is_some_and(|n| n.pending >= limits.per_net_pending)
        {
            return Err(Refused::NetPending);
        }
        if rate && source.tokens < 1.0 {
            return Err(Refused::Rate);
        }
        Ok(())
    }

    fn net(&self, loc: Loc) -> Option<&NetCount> {
        self.nets.get(&loc.net?)
    }

    /// Adds a pending connection from `ip`; a main-port one (`main`) takes a token and counts
    /// as a connection of its source at once.
    pub(crate) fn insert(&mut self, ip: IpAddr, tracked: bool, port: Port, main: bool) -> u64 {
        let id = self.next();
        let loc = Loc::of(ip);
        let counted = tracked.then_some(loc);
        if let Some(loc) = counted {
            self.count(loc, |s| s.pending += 1, |n| n.pending += 1);
            if main {
                self.count(
                    loc,
                    |s| {
                        s.tokens -= 1.0;
                        s.connections += 1;
                    },
                    |n| n.connections += 1,
                );
            }
        }
        let conn = Conn {
            class: Class::Pending,
            stamp: id,
            port,
            group: loc.key,
            pending_at: counted,
            conn_at: counted.filter(|_| main),
            token: CancellationToken::new(),
        };
        self.order.insert((Class::Pending, id, id));
        self.pending_set(port).add(loc.key, id);
        self.conns.insert(id, conn);
        id
    }

    fn count(
        &mut self,
        loc: Loc,
        source: impl FnOnce(&mut Source),
        net: impl FnOnce(&mut NetCount),
    ) {
        if let Some(s) = self.sources.get_mut(&loc.key) {
            source(s);
        }
        if let Some(key) = loc.net {
            net(self.nets.entry(key).or_default());
        }
    }

    fn uncount(&mut self, loc: Loc, pending: bool) {
        if let Some(s) = self.sources.get_mut(&loc.key) {
            let field = if pending {
                &mut s.pending
            } else {
                &mut s.connections
            };
            *field = field.saturating_sub(1);
        }
        let Some(key) = loc.net else { return };
        let Some(net) = self.nets.get_mut(&key) else {
            return;
        };
        let field = if pending {
            &mut net.pending
        } else {
            &mut net.connections
        };
        *field = field.saturating_sub(1);
        if net.pending == 0 && net.connections == 0 {
            self.nets.remove(&key);
        }
    }

    pub(crate) fn token(&self, id: u64) -> CancellationToken {
        self.conns
            .get(&id)
            .map(|c| c.token.clone())
            .unwrap_or_default()
    }

    /// A pending connection becomes established (idle); nothing for any other.
    pub(crate) fn establish(&mut self, id: u64) {
        if self
            .conns
            .get(&id)
            .is_some_and(|c| c.class == Class::Pending)
        {
            self.set_class(id, Class::Idle);
        }
    }

    /// Moves a connection to `class` with a fresh stamp (it was just active); leaving
    /// `Pending` releases its pending slot.
    pub(crate) fn set_class(&mut self, id: u64, class: Class) {
        let stamp = self.next();
        let Some(conn) = self.conns.get_mut(&id) else {
            return;
        };
        let (old, old_stamp, port, group) = (conn.class, conn.stamp, conn.port, conn.group);
        let released =
            (old == Class::Pending && class != Class::Pending).then(|| conn.pending_at.take());
        conn.class = class;
        conn.stamp = stamp;
        self.order.remove(&(old, old_stamp, id));
        self.order.insert((class, stamp, id));
        if let Some(pending_at) = released {
            self.pending_set(port).remove(group, id);
            if let Some(loc) = pending_at {
                self.uncount(loc, true);
            }
        }
    }

    /// Counts a websocket connection against the client `ip` once it is known.
    pub(crate) fn bind(&mut self, id: u64, ip: IpAddr, tracked: bool) {
        if !tracked || self.conns.get(&id).is_none_or(|c| c.conn_at.is_some()) {
            return;
        }
        let loc = Loc::of(ip);
        self.count(loc, |s| s.connections += 1, |n| n.connections += 1);
        if let Some(conn) = self.conns.get_mut(&id) {
            conn.conn_at = Some(loc);
        }
    }

    /// Releases everything a connection holds; nothing if it is already gone (evicted).
    pub(crate) fn remove(&mut self, id: u64) -> Option<CancellationToken> {
        let conn = self.conns.remove(&id)?;
        self.order.remove(&(conn.class, conn.stamp, id));
        if conn.class == Class::Pending {
            self.pending_set(conn.port).remove(conn.group, id);
        }
        if let Some(loc) = conn.pending_at {
            self.uncount(loc, true);
        }
        if let Some(loc) = conn.conn_at {
            self.uncount(loc, false);
        }
        Some(conn.token)
    }

    fn evict(&mut self, id: u64) -> bool {
        self.remove(id).map(|token| token.cancel()).is_some()
    }

    /// Closes the oldest pending connection of `port`'s heaviest source.
    pub(crate) fn evict_pending(&mut self, port: Port) -> bool {
        let victim = match port {
            Port::Main => self.main_pending.victim(),
            Port::Ws => self.ws_pending.victim(),
        };
        victim.is_some_and(|id| self.evict(id))
    }

    /// Closes the least valuable connection of all.
    pub(crate) fn evict_least_valuable(&mut self) -> bool {
        let victim = self.order.first().map(|&(_, _, id)| id);
        victim.is_some_and(|id| self.evict(id))
    }

    pub(crate) fn prune(&mut self, limits: &AcceptLimits, now: Instant) {
        let burst = f64::from(limits.per_ip_burst);
        self.sources.retain(|_, s| {
            s.refill(limits, now);
            s.pending > 0 || s.connections > 0 || s.tokens < burst
        });
    }

    #[cfg(test)]
    pub(crate) fn tracked(&self) -> usize {
        self.sources.len()
    }

    #[cfg(test)]
    pub(crate) fn nets(&self) -> usize {
        self.nets.len()
    }
}

impl Source {
    fn refill(&mut self, limits: &AcceptLimits, now: Instant) {
        let elapsed = now.saturating_duration_since(self.refilled).as_secs_f64();
        let rate = f64::from(limits.per_ip_per_second);
        self.tokens = (self.tokens + elapsed * rate).min(f64::from(limits.per_ip_burst));
        self.refilled = now;
    }
}
