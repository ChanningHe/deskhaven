//! The in-memory presence table (02 §3, 04 §4): which devices are online right now, fed by
//! the signal server's liveness notifications and by the client heartbeats. Never stored;
//! `devices.last_seen_at` is written from it in batches ([`Presence::take_unflushed`]).
//!
//! - **Online** means a signal registration within [`SIGNAL_TTL`] (hbbs `REG_TIMEOUT`, the
//!   window OnlineRequest and punch holes use, so the console agrees with what clients see)
//!   **or** a heartbeat within [`HEARTBEAT_TTL`] (three idle heartbeat periods of the client,
//!   `sync.rs` `TIME_HEARTBEAT` = 15 s). A signal channel that closed ends the signal half at
//!   once ([`Presence::signal_offline`]), as it does in hbbs.
//! - **Bounded**: at most `capacity` entries ([`DEFAULT_CAPACITY`]). A new id beyond that
//!   triggers an eviction pass (at most once per [`SWEEP_INTERVAL`]): first the entries that
//!   are offline and already flushed; if the table is still full, offline entries oldest
//!   contact first (identity-only entries before any) down to 90 % of the capacity, even
//!   unflushed ones, which only costs `last_seen_at` precision (logged). Online entries are
//!   never evicted: if they alone fill the table, the new id is not tracked (logged at warn at
//!   most once per minute) while every tracked id keeps updating. Each entry keeps at most
//!   [`MAX_CONNS`] connection ids. Inputs only come from ids the signal server holds a key for
//!   and from verified heartbeats, so the bound is a safety net, not the normal regime.
//! - **Flush**: nothing here writes to the database. The `last_seen_at` flush task
//!   ([`LastSeenFlusher`](super::LastSeenFlusher)) calls [`Presence::take_unflushed`] every
//!   `devices.last_seen_flush`, which also evicts the flushed offline entries.
//! - **Clocks**: liveness, the heartbeat interval and the sweep / warning timers run on the
//!   monotonic clock ([`Clock::elapsed_ms`](crate::clock::Clock::elapsed_ms)); only the
//!   reported `last_seen_ms` is wall time. A wall clock stepped back therefore neither keeps
//!   silent devices online nor refuses heartbeats until it catches up.
//! - **Cheap**: one mutex over a map, short critical sections, no I/O: the signal server
//!   calls in from its UDP loop.
//! - It also caches the verified identity of a device (row id and uuid, both immutable for a
//!   row) so heartbeat verification needs no query once a device is known; deleting the
//!   device drops it ([`Presence::forget`]). A lookup that started before a forget cannot
//!   re-insert what it read ([`Presence::identity_epoch`]).

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
    /// Connection ids of the latest heartbeat while it is within [`HEARTBEAT_TTL`]; empty
    /// once it expired, even if the signal registration keeps the device online.
    pub conns: Vec<i32>,
}

/// What [`Presence::live`] counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiveCounts {
    /// Devices online now.
    pub devices: usize,
    /// Connection ids those devices reported in their latest heartbeat, while still fresh.
    pub connections: usize,
}

/// A device seen since the previous [`Presence::take_unflushed`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    pub rustdesk_id: String,
    /// Latest contact (unix ms).
    pub last_seen_ms: i64,
    pub ip: Option<IpAddr>,
}

/// A verified device: its `devices` row id and uuid (both fixed for the row's lifetime), and
/// whether the row holds a sysinfo, which only ever turns true for a row (as read, then
/// [`Presence::mark_sysinfo`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Identity {
    pub device_id: i64,
    pub uuid: Vec<u8>,
    pub has_sysinfo: bool,
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
    /// Monotonic ms of the last eviction pass (`None`: never); the timers below likewise.
    last_sweep_ms: Option<i64>,
    last_full_warn_ms: Option<i64>,
    last_precision_warn_ms: Option<i64>,
    /// Number of [`Presence::forget`] calls so far.
    forgets: u64,
}

#[derive(Default)]
struct Entry {
    /// Monotonic ms ([`Clock::elapsed_ms`](crate::clock::Clock::elapsed_ms)) of the latest
    /// signal registration / heartbeat: liveness and the heartbeat interval.
    signal_at: Option<i64>,
    heartbeat_at: Option<i64>,
    /// Wall time (unix ms) of the latest contact: what `last_seen_at` stores.
    seen_wall: Option<i64>,
    ip: Option<IpAddr>,
    conns: Vec<i32>,
    /// Seen since the last flush.
    unflushed: bool,
    identity: Option<Identity>,
}

impl Entry {
    fn last_seen_ms(&self) -> Option<i64> {
        self.seen_wall
    }

    /// Monotonic ms of the latest contact (eviction order).
    fn last_contact(&self) -> Option<i64> {
        self.signal_at.max(self.heartbeat_at)
    }

    /// `now` is monotonic ms.
    fn is_online(&self, now: i64) -> bool {
        within(self.signal_at, SIGNAL_TTL, now) || self.heartbeat_is_fresh(now)
    }

    fn heartbeat_is_fresh(&self, now: i64) -> bool {
        within(self.heartbeat_at, HEARTBEAT_TTL, now)
    }

    /// The connections of the latest heartbeat while it is within [`HEARTBEAT_TTL`]: a device
    /// kept online by its signal registration alone reports none (`now` is monotonic ms).
    fn live_conns(&self, now: i64) -> &[i32] {
        if self.heartbeat_is_fresh(now) {
            &self.conns
        } else {
            &[]
        }
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
        let wall = self.wall();
        self.update(rustdesk_id, |entry, now| {
            entry.signal_at = Some(now);
            entry.seen_wall = Some(wall);
            entry.ip = Some(ip);
            entry.unflushed = true;
        });
    }

    /// The signal channel of `rustdesk_id` closed: its signal liveness ends now (a heartbeat
    /// may still keep it online). Unknown ids are ignored.
    pub fn signal_offline(&self, rustdesk_id: &str) {
        if let Some(entry) = self.lock().entries.get_mut(rustdesk_id) {
            entry.signal_at = None;
        }
    }

    /// A verified heartbeat of `rustdesk_id` from `ip` reporting the live connections
    /// `conns` (the first [`MAX_CONNS`] are kept).
    pub fn heartbeat(&self, rustdesk_id: &str, ip: IpAddr, conns: &[i32]) {
        self.try_heartbeat(rustdesk_id, ip, conns, Duration::ZERO);
    }

    /// [`heartbeat`](Self::heartbeat), unless the previous accepted heartbeat of `rustdesk_id`
    /// is less than `min_interval` ago: then `false` and nothing changes (06 §2.4). An id the
    /// table has no room for is never refused.
    pub fn try_heartbeat(
        &self,
        rustdesk_id: &str,
        ip: IpAddr,
        conns: &[i32],
        min_interval: Duration,
    ) -> bool {
        let (now, wall) = (self.now(), self.wall());
        let mut state = self.lock();
        let too_soon = |entry: &Entry| {
            let previous = entry.heartbeat_at;
            previous.is_some_and(|t| now.saturating_sub(t) < millis(min_interval))
        };
        let tracked = state.entries.get(rustdesk_id).map(too_soon);
        match tracked {
            Some(true) => return false,
            Some(false) => {}
            None if self.make_room(&mut state, now) => {
                state
                    .entries
                    .insert(rustdesk_id.to_owned(), Entry::default());
            }
            None => return true,
        }
        let Some(entry) = state.entries.get_mut(rustdesk_id) else {
            return true;
        };
        entry.heartbeat_at = Some(now);
        entry.seen_wall = Some(wall);
        entry.ip = Some(ip);
        entry.conns = conns.iter().take(MAX_CONNS).copied().collect();
        entry.unflushed = true;
        true
    }

    /// The device's sysinfo was stored: its cached identity (if any) now has one.
    pub(crate) fn mark_sysinfo(&self, rustdesk_id: &str) {
        let mut state = self.lock();
        let identity = state
            .entries
            .get_mut(rustdesk_id)
            .and_then(|entry| entry.identity.as_mut());
        if let Some(identity) = identity {
            identity.has_sysinfo = true;
        }
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
            conns: entry.live_conns(now).to_vec(),
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

    /// Online devices and their live connections, in one pass over the table. Connections
    /// count only while the heartbeat that reported them is fresh: what an offline device,
    /// or one whose heartbeats stopped while its signal channel lives on, reported is stale.
    pub fn live(&self) -> LiveCounts {
        let now = self.now();
        let state = self.lock();
        let online = state.entries.values().filter(|e| e.is_online(now));
        online.fold(LiveCounts::default(), |counts, entry| LiveCounts {
            devices: counts.devices + 1,
            connections: counts.connections + entry.live_conns(now).len(),
        })
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
        let mut state = self.lock();
        state.entries.remove(rustdesk_id);
        state.forgets += 1;
    }

    pub(crate) fn identity(&self, rustdesk_id: &str) -> Option<Identity> {
        let state = self.lock();
        state.entries.get(rustdesk_id)?.identity.clone()
    }

    /// Take before reading an identity from the database; pass to [`remember`](Self::remember).
    pub(crate) fn identity_epoch(&self) -> u64 {
        self.lock().forgets
    }

    /// Caches `identity` unless a [`forget`](Self::forget) happened since `epoch` was taken
    /// (the row read may be gone).
    pub(crate) fn remember(&self, rustdesk_id: &str, identity: Identity, epoch: u64) {
        let now = self.now();
        let mut state = self.lock();
        if state.forgets != epoch {
            return;
        }
        if let Some(entry) = state.entries.get_mut(rustdesk_id) {
            entry.identity = Some(identity);
        } else if self.make_room(&mut state, now) {
            let entry = Entry {
                identity: Some(identity),
                ..Entry::default()
            };
            state.entries.insert(rustdesk_id.to_owned(), entry);
        }
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
            if state.entries.len() >= capacity {
                let target = capacity.saturating_sub((capacity / 10).max(1));
                evict_oldest_offline(state, now, target);
            }
        }
        if state.entries.len() < capacity {
            return true;
        }
        if !recently(&mut state.last_full_warn_ms, now) {
            tracing::warn!(
                capacity,
                "device presence table full of online devices; new devices are not tracked"
            );
        }
        false
    }

    /// Monotonic ms: every in-memory window (liveness, interval, sweeps) uses it, so a stepped
    /// wall clock neither freezes nor stretches them.
    fn now(&self) -> i64 {
        self.inner.clock.elapsed_ms()
    }

    /// Wall ms, for the stored `last_seen_at` only.
    fn wall(&self) -> i64 {
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

/// Removes offline entries, those without any contact first, then oldest contact first,
/// until at most `target` entries remain (online entries are kept even above it).
fn evict_oldest_offline(state: &mut State, now: i64, target: usize) {
    let mut offline: Vec<(Option<i64>, String)> = state
        .entries
        .iter()
        .filter(|(_, e)| !e.is_online(now))
        .map(|(id, e)| (e.last_contact(), id.clone()))
        .collect();
    offline.sort_unstable();
    let excess = state.entries.len().saturating_sub(target);
    let mut unflushed = 0_usize;
    for (_, id) in offline.into_iter().take(excess) {
        if state.entries.remove(&id).is_some_and(|e| e.unflushed) {
            unflushed += 1;
        }
    }
    if unflushed > 0 && !recently(&mut state.last_precision_warn_ms, now) {
        tracing::warn!(
            unflushed,
            "device presence table full: evicted offline devices before their last_seen_at \
             was flushed"
        );
    }
}

/// Whether `last` is within [`FULL_WARN_INTERVAL_MS`] of `now`; if not, moves it to `now`.
fn recently(last: &mut Option<i64>, now: i64) -> bool {
    if last.is_some_and(|t| now - t < FULL_WARN_INTERVAL_MS) {
        return true;
    }
    *last = Some(now);
    false
}

fn millis(d: Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
}

/// `at` (monotonic ms) lies less than `ttl` before `now`.
fn within(at: Option<i64>, ttl: Duration, now: i64) -> bool {
    at.is_some_and(|t| now - t < millis(ttl))
}

#[cfg(test)]
mod tests;
