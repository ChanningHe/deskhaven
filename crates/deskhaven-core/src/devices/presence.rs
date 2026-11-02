//! The in-memory presence table (02 §3, 04 §4): which devices are online right now, fed by
//! the signal server's liveness notifications and by the client heartbeats. Never stored;
//! `devices.last_seen_at` is written from it in batches ([`Presence::take_unflushed`]).
//!
//! - **Online** means a signal registration within [`SIGNAL_TTL`] (hbbs `REG_TIMEOUT`, the
//!   window OnlineRequest and punch holes use, so the console agrees with what clients see)
//!   **or** a heartbeat within [`HEARTBEAT_TTL`] (three idle heartbeat periods of the client,
//!   `sync.rs` `TIME_HEARTBEAT` = 15 s). A signal channel that closed ends the signal half at
//!   once ([`Presence::signal_offline`]), as it does in hbbs.
//! - **Bounded**: at most `capacity` entries ([`DEFAULT_CAPACITY`]). A new id beyond that first
//!   evicts the entries that are offline and already flushed (at most once per
//!   [`SWEEP_INTERVAL`]); if none can go, the new id is not tracked (logged at warn at most once
//!   per minute) while every tracked id keeps updating. Each entry keeps at most
//!   [`MAX_CONNS`] connection ids. Inputs only come from ids the signal server holds a key for
//!   and from verified heartbeats, so the bound is a safety net, not the normal regime.
//! - **Cheap**: one mutex over a map, short critical sections, no I/O: the signal server
//!   calls in from its UDP loop.
//! - It also caches the verified identity of a device (row id and uuid, both immutable for a
//!   row) so heartbeat verification needs no query once a device is known; deleting the
//!   device drops it ([`Presence::forget`]).

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use crate::clock::SharedClock;

/// Signal liveness window: hbbs `REG_TIMEOUT` (rendezvous_server.rs).
pub const SIGNAL_TTL: Duration = Duration::from_secs(30);
/// Heartbeat liveness window: three idle heartbeat periods (15 s) of the client.
pub const HEARTBEAT_TTL: Duration = Duration::from_secs(45);
/// Tracked devices at most (ten times the 10,000-device target of 01 §NFR).
pub const DEFAULT_CAPACITY: usize = 100_000;
/// Connection ids kept per device (the client reports its live connections).
pub const MAX_CONNS: usize = 64;
/// At most one eviction pass per interval while the table is full.
pub const SWEEP_INTERVAL: Duration = Duration::from_secs(1);
/// At most one "table full" warning per interval.
const FULL_WARN_INTERVAL_MS: i64 = 60_000;

/// What [`Presence::get`] reports about a device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DevicePresence {
    pub online: bool,
    /// Latest signal registration or heartbeat (unix ms); `None` if only the identity is
    /// cached.
    pub last_seen_ms: Option<i64>,
    /// Source address of that latest contact.
    pub ip: Option<IpAddr>,
    /// Connection ids of the latest heartbeat.
    pub conns: Vec<i32>,
}

/// A device seen since the previous [`Presence::take_unflushed`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    pub rustdesk_id: String,
    /// Latest contact (unix ms).
    pub last_seen_ms: i64,
    pub ip: Option<IpAddr>,
}

/// A verified device: its `devices` row id and uuid (both fixed for the row's lifetime).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Identity {
    pub device_id: i64,
    pub uuid: Vec<u8>,
}

/// Shared handle; clones see the same table.
#[derive(Clone)]
pub struct Presence {
    inner: Arc<Inner>,
}

struct Inner {
    clock: SharedClock,
    capacity: usize,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    entries: HashMap<String, Entry>,
    /// Unix ms of the last eviction pass (`None`: never).
    last_sweep_ms: Option<i64>,
    last_full_warn_ms: Option<i64>,
}

#[derive(Default)]
struct Entry {
    signal_ms: Option<i64>,
    heartbeat_ms: Option<i64>,
    ip: Option<IpAddr>,
    conns: Vec<i32>,
    /// Seen since the last flush.
    unflushed: bool,
    identity: Option<Identity>,
}

impl Entry {
    fn last_seen_ms(&self) -> Option<i64> {
        self.signal_ms.max(self.heartbeat_ms)
    }

    fn is_online(&self, now: i64) -> bool {
        let within = |at: Option<i64>, ttl: Duration| at.is_some_and(|t| now - t < millis(ttl));
        within(self.signal_ms, SIGNAL_TTL) || within(self.heartbeat_ms, HEARTBEAT_TTL)
    }

    /// Nothing would be lost by dropping it.
    fn is_evictable(&self, now: i64) -> bool {
        !self.unflushed && !self.is_online(now)
    }
}

impl Presence {
    pub fn new(clock: SharedClock) -> Self {
        Self::with_capacity(clock, DEFAULT_CAPACITY)
    }

    pub fn with_capacity(clock: SharedClock, capacity: usize) -> Self {
        Self {
            inner: Arc::new(Inner {
                clock,
                capacity,
                state: Mutex::new(State::default()),
            }),
        }
    }

    /// The signal server refreshed the registration of `rustdesk_id`, seen from `ip`.
    pub fn signal_online(&self, rustdesk_id: &str, ip: IpAddr) {
        self.update(rustdesk_id, |entry, now| {
            entry.signal_ms = Some(now);
            entry.ip = Some(ip);
            entry.unflushed = true;
        });
    }

    /// The signal channel of `rustdesk_id` closed: its signal liveness ends now (a heartbeat
    /// may still keep it online). Unknown ids are ignored.
    pub fn signal_offline(&self, rustdesk_id: &str) {
        if let Some(entry) = self.lock().entries.get_mut(rustdesk_id) {
            entry.signal_ms = None;
        }
    }

    /// A verified heartbeat of `rustdesk_id` from `ip` reporting the live connections
    /// `conns` (the first [`MAX_CONNS`] are kept).
    pub fn heartbeat(&self, rustdesk_id: &str, ip: IpAddr, conns: &[i32]) {
        self.update(rustdesk_id, |entry, now| {
            entry.heartbeat_ms = Some(now);
            entry.ip = Some(ip);
            entry.conns = conns.iter().take(MAX_CONNS).copied().collect();
            entry.unflushed = true;
        });
    }

    /// What is known about `rustdesk_id`; `None` if it is not tracked.
    pub fn get(&self, rustdesk_id: &str) -> Option<DevicePresence> {
        let now = self.now();
        let state = self.lock();
        let entry = state.entries.get(rustdesk_id)?;
        Some(DevicePresence {
            online: entry.is_online(now),
            last_seen_ms: entry.last_seen_ms(),
            ip: entry.ip,
            conns: entry.conns.clone(),
        })
    }

    pub fn is_online(&self, rustdesk_id: &str) -> bool {
        self.get(rustdesk_id).is_some_and(|p| p.online)
    }

    /// Devices online now (a pass over the table).
    pub fn online_count(&self) -> usize {
        let now = self.now();
        let state = self.lock();
        state.entries.values().filter(|e| e.is_online(now)).count()
    }

    /// Tracked entries (online or not).
    pub fn len(&self) -> usize {
        self.lock().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Every device seen since the previous call, with its latest contact, for the
    /// `last_seen_at` flush; also evicts the entries that are offline and already flushed.
    /// A caller whose write fails should report the result again later; nothing is retried
    /// here.
    pub fn take_unflushed(&self) -> Vec<Seen> {
        let now = self.now();
        let mut state = self.lock();
        let mut seen = Vec::new();
        for (id, entry) in state.entries.iter_mut().filter(|(_, e)| e.unflushed) {
            entry.unflushed = false;
            if let Some(last_seen_ms) = entry.last_seen_ms() {
                seen.push(Seen {
                    rustdesk_id: id.clone(),
                    last_seen_ms,
                    ip: entry.ip,
                });
            }
        }
        state.entries.retain(|_, e| !e.is_evictable(now));
        state.last_sweep_ms = Some(now);
        seen
    }

    /// Drops everything about `rustdesk_id` (its device was deleted).
    pub fn forget(&self, rustdesk_id: &str) {
        self.lock().entries.remove(rustdesk_id);
    }

    pub(crate) fn identity(&self, rustdesk_id: &str) -> Option<Identity> {
        let state = self.lock();
        state.entries.get(rustdesk_id)?.identity.clone()
    }

    pub(crate) fn remember(&self, rustdesk_id: &str, identity: Identity) {
        self.update(rustdesk_id, |entry, _| entry.identity = Some(identity));
    }

    /// Applies `change` to the entry of `rustdesk_id`, creating it if the table has room.
    fn update(&self, rustdesk_id: &str, change: impl FnOnce(&mut Entry, i64)) {
        let now = self.now();
        let mut state = self.lock();
        if let Some(entry) = state.entries.get_mut(rustdesk_id) {
            change(entry, now);
            return;
        }
        if !self.make_room(&mut state, now) {
            return;
        }
        let mut entry = Entry::default();
        change(&mut entry, now);
        state.entries.insert(rustdesk_id.to_owned(), entry);
    }

    /// `true` if one more entry fits, evicting what can go when the table is full.
    fn make_room(&self, state: &mut State, now: i64) -> bool {
        let capacity = self.inner.capacity;
        if state.entries.len() < capacity {
            return true;
        }
        let swept_recently = state
            .last_sweep_ms
            .is_some_and(|t| now - t < millis(SWEEP_INTERVAL));
        if !swept_recently {
            state.entries.retain(|_, e| !e.is_evictable(now));
            state.last_sweep_ms = Some(now);
        }
        if state.entries.len() < capacity {
            return true;
        }
        let warned_recently = state
            .last_full_warn_ms
            .is_some_and(|t| now - t < FULL_WARN_INTERVAL_MS);
        if !warned_recently {
            state.last_full_warn_ms = Some(now);
            tracing::warn!(
                capacity,
                "device presence table full; new devices are not tracked"
            );
        }
        false
    }

    fn now(&self) -> i64 {
        self.inner.clock.now_ms()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // Every update leaves the map consistent, so a panicked holder harms nothing.
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

fn millis(d: Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests;
