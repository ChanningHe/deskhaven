//! Admission control of the main TCP and websocket ports, decided at accept time before the
//! KeyExchange offer (an X25519 key and two Ed25519 signatures) is computed.
//!
//! Per-source limits refuse (they only hurt that source); global caps never refuse but evict
//! the least valuable connection (pending handshakes first, then unregistered idle ones,
//! registered peers last), so a flood can crowd a new client but never lock it out.

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

static REFUSED_LOG: LogLimit = LogLimit::new(Duration::from_secs(60));
static EVICTED_LOG: LogLimit = LogLimit::new(Duration::from_secs(60));

/// Share of `RLIMIT_NOFILE` for connections of these ports (the HTTP listeners take half).
const NOFILE_CONNECTIONS_DIVISOR: u64 = 4;
/// The ceiling keeps memory sane on hosts with a huge open-file limit.
const MIN_CONNECTIONS: u32 = 64;
const MAX_CONNECTIONS: u32 = 32_768;
/// Pending budgets as fractions of the connection cap (both at most half of it).
const PENDING_DIVISOR: u32 = 4;
const WS_PENDING_DIVISOR: u32 = 8;

/// A per-source limit of zero refuses everything it covers; a global cap of zero refuses
/// everything too (nothing to evict).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AcceptLimits {
    pub per_ip_burst: u32,
    pub per_ip_per_second: u32,
    pub per_ip_pending: u32,
    pub per_net_pending: u32,
    pub per_ip_connections: u32,
    pub per_net_connections: u32,
    /// Main-port connections in all that have not sent their first frame.
    pub pending: u32,
    /// Websocket connections in all that have not finished their handshake.
    pub ws_pending: u32,
    pub connections: u32,
}

impl Default for AcceptLimits {
    /// Sized so RustDesk clients behind one NAT address are not throttled noticeably; one
    /// source refilling at 64/s spends well under 1% of a core on offers.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    SourceConnections,
    NetConnections,
    Rate,
    SourcePending,
    NetPending,
    /// A global cap is zero.
    Disabled,
}

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

    pub(crate) fn admit_logged(&self, ip: IpAddr) -> Option<Slot> {
        self.admit(ip, Instant::now())
            .map_err(|refused| log_refused(&format!("main-port connection from {ip}"), refused))
            .ok()
    }

    pub(crate) fn admit_ws_logged(&self, peer: IpAddr, trusted_proxy: bool) -> Option<Slot> {
        self.admit_ws(peer, trusted_proxy, Instant::now())
            .map_err(|refused| log_refused(&format!("websocket connection from {peer}"), refused))
            .ok()
    }

    pub(crate) fn admit(&self, ip: IpAddr, now: Instant) -> Result<Slot, Refused> {
        let mut state = self.lock();
        let tracked = state.track(ip, &self.limits, now);
        state.check_connections(ip, tracked, &self.limits)?;
        state.check_pending(ip, tracked, &self.limits, true)?;
        self.make_room(&mut state, Port::Main)?;
        let id = state.insert(ip, tracked, Port::Main, true);
        Ok(self.slot(&state, id))
    }

    /// A trusted proxy's connections skip the per-source pending limits (they carry many clients).
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

    pub(crate) fn prune(&self, now: Instant) {
        self.lock().prune(&self.limits, now);
    }
}

impl Slot {
    /// Cancelled when the gate closes this connection to make room.
    pub(crate) fn evicted(&self) -> CancellationToken {
        self.evicted.clone()
    }

    pub(crate) fn established(&self) {
        lock(&self.state).establish(self.id);
    }

    pub(crate) fn active(&self, registered: bool) {
        let class = if registered { Class::Peer } else { Class::Idle };
        lock(&self.state).set_class(self.id, class);
    }

    /// `false` (the connection must close) when that source or its /48 is at its connection limit.
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
