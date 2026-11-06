//! Admission control of the main TCP port and the websocket port (deskhaven code, not
//! vendored; M4-05).
//!
//! Each accepted main-port connection gets a KeyExchange offer at once (an X25519 key
//! generation and two Ed25519 signatures, about 100 µs), before the client has sent anything.
//! The gate decides at accept time, before any of that work, whether a connection may start.
//!
//! One principle: **per-source limits refuse** (they only hurt that source), **global caps
//! never refuse**; when one is full, the least valuable existing connection is closed instead
//! (its cancel token fires), so no flood can keep a new client out, only crowd it.
//!
//! Per source (an IPv4 address, IPv4-mapped IPv6 included, or an IPv6 /64) and per IPv6 /48:
//!
//! - **accept rate** (main port): a token bucket of `per_ip_burst` refilled at
//!   `per_ip_per_second`, bounding the CPU one source spends on offers;
//! - **pending** (main port, and websocket connections before their handshake unless the peer
//!   is a trusted proxy): `per_ip_pending` per source, `per_net_pending` per /48;
//! - **connections** (both ports, established or pending): `per_ip_connections` per source,
//!   `per_net_connections` per /48. A websocket connection counts here once its handshake
//!   resolved the client address (behind a proxy the source is only known then).
//!
//! Global, by eviction:
//!
//! - `pending` main-port and `ws_pending` websocket handshakes: at the cap, the oldest pending
//!   connection of the source holding the most pending ones is closed (an attacker with many
//!   addresses can still crowd slow handshakes out, never lock the port);
//! - `connections` in all: at the cap, the least valuable connection goes: a pending
//!   handshake, oldest first; then an established connection without a registered peer, the
//!   longest idle first; registered peers' connections last.
//!
//! [`Slot`] releases everything it holds when dropped.

mod loc;
mod pending_set;
mod state;
#[cfg(test)]
mod tests;

use std::net::IpAddr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use hbb_common::log;
use tokio_util::sync::CancellationToken;

use crate::log_limit::LogLimit;
use state::{Class, Port, State};

pub(crate) use loc::source_key;

/// Refusals and evictions are logged at most once a minute each, with their count.
static REFUSED_LOG: LogLimit = LogLimit::new(Duration::from_secs(60));
static EVICTED_LOG: LogLimit = LogLimit::new(Duration::from_secs(60));

/// Share of `RLIMIT_NOFILE` for connections of these ports (the HTTP listeners take half).
const NOFILE_CONNECTIONS_DIVISOR: u64 = 4;
/// Bounds of the scaled connection cap. The ceiling keeps memory sane on hosts with a huge
/// open-file limit (each connection holds a task and its buffers, tens of KiB).
const MIN_CONNECTIONS: u32 = 64;
const MAX_CONNECTIONS: u32 = 32_768;
/// Pending budgets as fractions of the connection cap (both at most half of it).
const PENDING_DIVISOR: u32 = 4;
const WS_PENDING_DIVISOR: u32 = 8;

/// Limits of [`AcceptGate`]; see the module docs. A per-source limit of zero refuses
/// everything it covers; a global cap of zero refuses everything too (nothing to evict).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AcceptLimits {
    /// Connections one source may open at once before its bucket is empty.
    pub per_ip_burst: u32,
    /// Bucket refill per second, per source.
    pub per_ip_per_second: u32,
    /// Pending connections of one source.
    pub per_ip_pending: u32,
    /// Pending connections of one IPv6 /48 (all its /64s together).
    pub per_net_pending: u32,
    /// Connections of one source, established or pending.
    pub per_ip_connections: u32,
    /// Connections of one IPv6 /48, established or pending.
    pub per_net_connections: u32,
    /// Main-port connections in all that have not sent their first frame.
    pub pending: u32,
    /// Websocket connections in all that have not finished their handshake.
    pub ws_pending: u32,
    /// Connections in all (main and websocket port), established or pending.
    pub connections: u32,
}

impl Default for AcceptLimits {
    /// Sized so the RustDesk clients behind one NAT address (TCP-mode devices reconnecting
    /// after a server restart, controllers of a support desk) are not throttled noticeably:
    /// connections normally send their first frame within a round trip of the offer, and one
    /// source refilling at 64/s spends well under 1% of a core on offers. The global caps are
    /// those of an open-file limit of 65 536 ([`AcceptLimits::for_open_files`]).
    fn default() -> Self {
        Self {
            per_ip_burst: 256,
            per_ip_per_second: 64,
            per_ip_pending: 64,
            per_net_pending: 256,
            per_ip_connections: 1024,
            per_net_connections: 4096,
            pending: 4096,
            ws_pending: 2048,
            connections: 16_384,
        }
    }
}

impl AcceptLimits {
    /// The defaults with the global caps scaled to the open-file soft limit (`None`: unknown
    /// or unlimited, keep the defaults): connections a quarter of it, within 64..=32 768;
    /// pending a quarter and websocket pending an eighth of the connection cap.
    pub fn for_open_files(soft_limit: Option<u64>) -> Self {
        let base = Self::default();
        let Some(limit) = soft_limit else {
            return base;
        };
        let share = u32::try_from(limit / NOFILE_CONNECTIONS_DIVISOR).unwrap_or(u32::MAX);
        let connections = share.clamp(MIN_CONNECTIONS, MAX_CONNECTIONS);
        Self {
            connections,
            pending: connections / PENDING_DIVISOR,
            ws_pending: connections / WS_PENDING_DIVISOR,
            ..base
        }
    }
}

/// Why a connection was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    /// The source has `per_ip_connections` connections.
    SourceConnections,
    /// The source's IPv6 /48 has `per_net_connections` connections.
    NetConnections,
    /// The source's accept budget is used up.
    Rate,
    /// The source has `per_ip_pending` connections pending.
    SourcePending,
    /// The source's IPv6 /48 has `per_net_pending` connections pending.
    NetPending,
    /// A global cap is zero.
    Disabled,
}

/// Shared admission state; cheap to clone.
#[derive(Clone)]
pub(crate) struct AcceptGate {
    limits: AcceptLimits,
    state: Arc<Mutex<State>>,
}

/// A connection's place in the gate; dropping it releases everything it holds.
pub(crate) struct Slot {
    state: Arc<Mutex<State>>,
    limits: AcceptLimits,
    id: u64,
    evicted: CancellationToken,
}

impl AcceptGate {
    pub(crate) fn new(limits: AcceptLimits) -> Self {
        Self {
            limits,
            state: Arc::default(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    /// [`AcceptGate::admit`] now; a refusal is logged (at most once a minute, with a count).
    pub(crate) fn admit_logged(&self, ip: IpAddr) -> Option<Slot> {
        self.admit(ip, Instant::now())
            .map_err(|refused| log_refused(&format!("main-port connection from {ip}"), refused))
            .ok()
    }

    /// [`AcceptGate::admit_ws`] now, logged like [`AcceptGate::admit_logged`].
    pub(crate) fn admit_ws_logged(&self, peer: IpAddr, trusted_proxy: bool) -> Option<Slot> {
        self.admit_ws(peer, trusted_proxy, Instant::now())
            .map_err(|refused| log_refused(&format!("websocket connection from {peer}"), refused))
            .ok()
    }

    /// Admits a main-port connection from `ip` at `now`, or says why not.
    pub(crate) fn admit(&self, ip: IpAddr, now: Instant) -> Result<Slot, Refused> {
        let mut state = self.lock();
        let tracked = state.track(ip, &self.limits, now);
        state.check_connections(ip, tracked, &self.limits)?;
        state.check_pending(ip, tracked, &self.limits, true)?;
        self.make_room(&mut state, Port::Main)?;
        let id = state.insert(ip, tracked, Port::Main, true);
        Ok(self.slot(&state, id))
    }

    /// Admits a websocket connection from the socket peer `peer` at `now`. A trusted proxy's
    /// connections skip the per-source pending limits (they carry many clients); they still
    /// share the websocket pending budget.
    pub(crate) fn admit_ws(
        &self,
        peer: IpAddr,
        trusted_proxy: bool,
        now: Instant,
    ) -> Result<Slot, Refused> {
        let mut state = self.lock();
        let tracked = !trusted_proxy && state.track(peer, &self.limits, now);
        if !trusted_proxy {
            state.check_pending(peer, tracked, &self.limits, false)?;
        }
        self.make_room(&mut state, Port::Ws)?;
        let id = state.insert(peer, tracked, Port::Ws, false);
        Ok(self.slot(&state, id))
    }

    /// Evicts what the global caps need for one more pending connection on `port`.
    fn make_room(&self, state: &mut State, port: Port) -> Result<(), Refused> {
        let budget = match port {
            Port::Main => self.limits.pending,
            Port::Ws => self.limits.ws_pending,
        };
        if budget == 0 || self.limits.connections == 0 {
            return Err(Refused::Disabled);
        }
        let mut evicted = false;
        if state.pending_len(port) >= budget as usize {
            evicted |= state.evict_pending(port);
        }
        while state.len() >= self.limits.connections as usize && state.evict_least_valuable() {
            evicted = true;
        }
        if evicted && let Some(count) = EVICTED_LOG.hit(Instant::now()) {
            log::warn!("closed {count} connection(s) to make room at a global cap");
        }
        Ok(())
    }

    fn slot(&self, state: &State, id: u64) -> Slot {
        Slot {
            state: Arc::clone(&self.state),
            limits: self.limits,
            id,
            evicted: state.token(id),
        }
    }

    /// Forgets sources with no connection and a full bucket.
    pub(crate) fn prune(&self, now: Instant) {
        self.lock().prune(&self.limits, now);
    }
}

impl Slot {
    /// Cancelled when the gate closes this connection to make room.
    pub(crate) fn evicted(&self) -> CancellationToken {
        self.evicted.clone()
    }

    /// The first frame arrived (main port) or the handshake finished (websocket): the
    /// connection no longer holds a pending slot.
    pub(crate) fn established(&self) {
        lock(&self.state).establish(self.id);
    }

    /// The connection did something useful; `registered`: it is a registered peer's channel.
    pub(crate) fn active(&self, registered: bool) {
        let class = if registered { Class::Peer } else { Class::Idle };
        lock(&self.state).set_class(self.id, class);
    }

    /// Counts a websocket connection against the client `ip` its handshake resolved; `false`
    /// (the connection must close) when that source or its /48 is at its connection limit.
    pub(crate) fn bind_source(&self, ip: IpAddr) -> bool {
        let mut state = lock(&self.state);
        let tracked = state.track(ip, &self.limits, Instant::now());
        if let Err(refused) = state.check_connections(ip, tracked, &self.limits) {
            log_refused(&format!("websocket connection from {ip}"), refused);
            return false;
        }
        state.bind(self.id, ip, tracked);
        true
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        lock(&self.state).remove(self.id);
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

fn log_refused(what: &str, refused: Refused) {
    if let Some(count) = REFUSED_LOG.hit(Instant::now()) {
        log::warn!("refused {count} connection(s); latest a {what}: {refused:?}");
    }
}
