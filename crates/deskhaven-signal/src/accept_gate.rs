//! Admission control of the main TCP port and the websocket port (deskhaven code, not
//! vendored; M4-05).
//!
//! Each accepted main-port connection gets a KeyExchange offer at once (an X25519 key
//! generation and two Ed25519 signatures, about 100 µs), before the client has sent anything.
//! The gate decides at accept time, before any of that work, whether a connection may start.
//! What it bounds, and nothing more:
//!
//! - **connections** (main and websocket port, established or pending): at most
//!   `per_ip_connections` per source and `connections` in all, so neither one source nor a
//!   flood can run the process out of file descriptors through these ports. A websocket
//!   connection counts in all from its accept and per source from its handshake (behind a
//!   proxy the source is only known then);
//! - **accept rate** (main port): a token bucket of `per_ip_burst` connections refilled at
//!   `per_ip_per_second` per source, bounding the CPU one source spends on offers;
//! - **pending handshakes** (main port, connections that have not sent their first frame): at
//!   most `per_ip_pending` per source and `per_net_pending` per IPv6 /48. When `pending` are
//!   pending in all, the **oldest** one is closed to admit the new one instead of refusing it:
//!   clients send their first frame within a round trip of the offer, so silent connections
//!   cannot keep anyone out (review H-A).
//!
//! Sources are IPv4 addresses (IPv4-mapped IPv6 included) and IPv6 /64s, which a single host
//! usually controls in full. A refused connection is simply closed. [`Connection`] and
//! [`Pending`] release their slot when dropped.

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, Ipv6Addr};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use hbb_common::log;
use tokio_util::sync::CancellationToken;

use crate::log_limit::LogLimit;

/// Refusals and evictions are logged at most once a minute each, with their count.
static REFUSED_LOG: LogLimit = LogLimit::new(Duration::from_secs(60));
static EVICTED_LOG: LogLimit = LogLimit::new(Duration::from_secs(60));

/// Most sources tracked at once. Beyond it, unknown sources skip the per-source limits (the
/// global caps still apply) rather than being refused, so a flood from many addresses cannot
/// lock new legitimate sources out. Idle sources are pruned on the server's timer.
const MAX_TRACKED: usize = 100_000;
/// Share of `RLIMIT_NOFILE` for connections of these ports (the HTTP listeners take half).
const NOFILE_CONNECTIONS_DIVISOR: u64 = 4;
/// Share of `RLIMIT_NOFILE` for pending handshakes.
const NOFILE_PENDING_DIVISOR: u64 = 16;

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
    /// Connections in all that have not sent their first frame; beyond it the oldest is closed.
    pub pending: u32,
    /// Pending connections of one IPv6 /48 (all its /64s together).
    pub per_net_pending: u32,
    /// Connections of one source, established or pending.
    pub per_ip_connections: u32,
    /// Connections in all (main and websocket port), established or pending.
    pub connections: u32,
}

impl Default for AcceptLimits {
    /// Sized so the RustDesk clients behind one NAT address (TCP-mode devices reconnecting
    /// after a server restart, controllers of a support desk) are not throttled noticeably:
    /// connections normally send their first frame within a round trip of the offer, so
    /// pending counts stay far below the caps, and one source refilling at 64/s spends well
    /// under 1% of a core on offers. The global caps fit an open-file limit of 65 536.
    fn default() -> Self {
        Self {
            per_ip_burst: 256,
            per_ip_per_second: 64,
            per_ip_pending: 64,
            pending: 2048,
            per_net_pending: 256,
            per_ip_connections: 1024,
            connections: 16_384,
        }
    }
}

impl AcceptLimits {
    /// The defaults with the global caps scaled to the open-file soft limit (`None`: unknown
    /// or unlimited, keep the defaults): a quarter of it for connections, a sixteenth for
    /// pending handshakes, never below 1024 connections and 2048 pending.
    pub fn for_open_files(soft_limit: Option<u64>) -> Self {
        let base = Self::default();
        let Some(limit) = soft_limit else {
            return base;
        };
        let share = |divisor: u64, floor: u32| {
            u32::try_from(limit / divisor)
                .unwrap_or(u32::MAX)
                .max(floor)
        };
        Self {
            connections: share(NOFILE_CONNECTIONS_DIVISOR, 1024),
            pending: share(NOFILE_PENDING_DIVISOR, base.pending),
            ..base
        }
    }
}

/// Why a connection was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    /// The server has `connections` connections.
    Connections,
    /// The source has `per_ip_connections` connections.
    SourceConnections,
    /// The source's accept budget is used up.
    Rate,
    /// The source has `per_ip_pending` connections pending.
    SourcePending,
    /// The source's IPv6 /48 has `per_net_pending` connections pending.
    NetPending,
    /// `pending` is zero.
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
    connections: u32,
    next_seq: u64,
    /// Pending handshakes by admission order (the first is the oldest).
    pending: BTreeMap<u64, PendingEntry>,
    sources: HashMap<IpAddr, Source>,
    /// Pending handshakes per IPv6 /48; an entry exists only while its count is non-zero.
    nets: HashMap<IpAddr, u32>,
}

struct PendingEntry {
    key: Option<IpAddr>,
    net: Option<IpAddr>,
    evict: CancellationToken,
}

struct Source {
    tokens: f64,
    refilled: Instant,
    pending: u32,
    connections: u32,
}

/// A connection slot; dropping it releases it.
pub(crate) struct Connection {
    state: Arc<Mutex<State>>,
    /// The source it counts against, once known and if tracked.
    key: Option<IpAddr>,
}

/// A pending handshake slot; dropping it releases it, unless it was evicted meanwhile.
pub(crate) struct Pending {
    state: Arc<Mutex<State>>,
    seq: u64,
    evicted: CancellationToken,
}

/// What a main-port connection holds from the gate.
pub(crate) struct Admitted {
    pub(crate) connection: Connection,
    pub(crate) pending: Pending,
}

impl AcceptGate {
    pub(crate) fn new(limits: AcceptLimits) -> Self {
        Self {
            limits,
            state: Arc::default(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// [`AcceptGate::admit`] now; a refusal is logged (at most once a minute, with a count).
    pub(crate) fn admit_logged(&self, ip: IpAddr) -> Option<Admitted> {
        self.admit(ip, Instant::now())
            .map_err(|refused| log_refused(&format!("main-port connection from {ip}"), refused))
            .ok()
    }

    /// [`AcceptGate::open`]; a refusal is logged like [`AcceptGate::admit_logged`]'s.
    pub(crate) fn open_logged(&self) -> Option<Connection> {
        self.open()
            .map_err(|refused| log_refused("websocket connection", refused))
            .ok()
    }

    /// Admits a main-port connection from `ip` at `now`, or says why not.
    pub(crate) fn admit(&self, ip: IpAddr, now: Instant) -> Result<Admitted, Refused> {
        let mut state = self.lock();
        if state.connections >= self.limits.connections {
            return Err(Refused::Connections);
        }
        if self.limits.pending == 0 {
            return Err(Refused::Pending);
        }
        let (key, net) = (source_key(ip), net_key(ip));
        let tracked = state.sources.contains_key(&key) || state.sources.len() < MAX_TRACKED;
        if tracked {
            let source = state.source(key, &self.limits, now);
            self.check_source(source)?;
        }
        if let Some(net) = net
            && state.nets.get(&net).copied().unwrap_or(0) >= self.limits.per_net_pending
        {
            return Err(Refused::NetPending);
        }
        if state.pending.len() >= self.limits.pending as usize {
            state.evict_oldest();
        }
        Ok(state.commit(&self.state, tracked.then_some(key), net))
    }

    /// Opens a websocket connection slot (in all only; see [`Connection::bind_source`]).
    pub(crate) fn open(&self) -> Result<Connection, Refused> {
        let mut state = self.lock();
        if state.connections >= self.limits.connections {
            return Err(Refused::Connections);
        }
        state.connections += 1;
        Ok(Connection {
            state: Arc::clone(&self.state),
            key: None,
        })
    }

    /// Whether `source` may open a pending connection now (bucket refilled first).
    fn check_source(&self, source: &mut Source) -> Result<(), Refused> {
        if source.connections >= self.limits.per_ip_connections {
            return Err(Refused::SourceConnections);
        }
        if source.pending >= self.limits.per_ip_pending {
            return Err(Refused::SourcePending);
        }
        if source.tokens < 1.0 {
            return Err(Refused::Rate);
        }
        Ok(())
    }

    /// Forgets sources with no connection and a full bucket: they are indistinguishable from
    /// sources never seen.
    pub(crate) fn prune(&self, now: Instant) {
        let limits = self.limits;
        let mut state = self.lock();
        state.sources.retain(|_, source| {
            source.refill(&limits, now);
            source.pending > 0 || source.connections > 0 || source.tokens < source.burst(&limits)
        });
    }
}

impl State {
    fn source(&mut self, key: IpAddr, limits: &AcceptLimits, now: Instant) -> &mut Source {
        let source = self.sources.entry(key).or_insert_with(|| Source {
            tokens: f64::from(limits.per_ip_burst),
            refilled: now,
            pending: 0,
            connections: 0,
        });
        source.refill(limits, now);
        source
    }

    /// Takes the slots of an admitted main-port connection (checked by the caller).
    fn commit(
        &mut self,
        shared: &Arc<Mutex<State>>,
        key: Option<IpAddr>,
        net: Option<IpAddr>,
    ) -> Admitted {
        if let Some(source) = key.and_then(|key| self.sources.get_mut(&key)) {
            source.tokens -= 1.0;
            source.pending += 1;
            source.connections += 1;
        }
        if let Some(net) = net {
            *self.nets.entry(net).or_default() += 1;
        }
        self.connections += 1;
        let (seq, evict) = (self.next_seq, CancellationToken::new());
        self.next_seq += 1;
        let entry = PendingEntry {
            key,
            net,
            evict: evict.clone(),
        };
        self.pending.insert(seq, entry);
        let connection = Connection {
            state: Arc::clone(shared),
            key,
        };
        let pending = Pending {
            state: Arc::clone(shared),
            seq,
            evicted: evict,
        };
        Admitted {
            connection,
            pending,
        }
    }

    /// Closes the oldest pending connection (its task sees the token) and frees its slot.
    fn evict_oldest(&mut self) {
        let Some((_, entry)) = self.pending.pop_first() else {
            return;
        };
        self.release(&entry);
        entry.evict.cancel();
        if let Some(count) = EVICTED_LOG.hit(Instant::now()) {
            log::warn!("closed {count} oldest pending main-port connection(s) at the cap");
        }
    }

    fn release(&mut self, entry: &PendingEntry) {
        if let Some(source) = entry.key.and_then(|key| self.sources.get_mut(&key)) {
            source.pending = source.pending.saturating_sub(1);
        }
        if let Some(net) = entry.net
            && let Some(count) = self.nets.get_mut(&net)
        {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.nets.remove(&net);
            }
        }
    }
}

impl Source {
    fn burst(&self, limits: &AcceptLimits) -> f64 {
        f64::from(limits.per_ip_burst)
    }

    fn refill(&mut self, limits: &AcceptLimits, now: Instant) {
        let elapsed = now.saturating_duration_since(self.refilled).as_secs_f64();
        let rate = f64::from(limits.per_ip_per_second);
        self.tokens = (self.tokens + elapsed * rate).min(self.burst(limits));
        self.refilled = now;
    }
}

impl Connection {
    /// Counts a websocket connection against `ip`'s source, once the handshake resolved it;
    /// `false` (the connection must close) when the source is at `per_ip_connections`.
    pub(crate) fn bind_source(&mut self, gate: &AcceptGate, ip: IpAddr) -> bool {
        let key = source_key(ip);
        let mut state = gate.lock();
        if !state.sources.contains_key(&key) && state.sources.len() >= MAX_TRACKED {
            return true;
        }
        let source = state.source(key, &gate.limits, Instant::now());
        if source.connections >= gate.limits.per_ip_connections {
            log_refused(
                &format!("websocket connection from {ip}"),
                Refused::SourceConnections,
            );
            return false;
        }
        source.connections += 1;
        self.key = Some(key);
        true
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.connections = state.connections.saturating_sub(1);
        if let Some(source) = self.key.and_then(|key| state.sources.get_mut(&key)) {
            source.connections = source.connections.saturating_sub(1);
        }
    }
}

impl Pending {
    /// Cancelled when the gate closes this connection to make room for a newer one.
    pub(crate) fn evicted(&self) -> CancellationToken {
        self.evicted.clone()
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(entry) = state.pending.remove(&self.seq) {
            state.release(&entry);
        }
    }
}

fn log_refused(what: &str, refused: Refused) {
    if let Some(count) = REFUSED_LOG.hit(Instant::now()) {
        log::warn!("refused {count} connection(s); latest a {what}: {refused:?}");
    }
}

/// The address itself for IPv4 (IPv4-mapped IPv6 included), its /64 for IPv6.
pub(crate) fn source_key(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(v6) => prefix(v6, 64),
        v4 => v4,
    }
}

/// The /48 of an IPv6 address; `None` for IPv4.
fn net_key(ip: IpAddr) -> Option<IpAddr> {
    match ip.to_canonical() {
        IpAddr::V6(v6) => Some(prefix(v6, 48)),
        IpAddr::V4(_) => None,
    }
}

fn prefix(ip: Ipv6Addr, len: u32) -> IpAddr {
    let mask = u128::MAX.checked_shl(128 - len).unwrap_or(0);
    IpAddr::V6(Ipv6Addr::from(u128::from(ip) & mask))
}
