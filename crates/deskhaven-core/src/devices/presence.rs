//! Online: a signal registration within [`SIGNAL_TTL`] or a heartbeat within [`HEARTBEAT_TTL`].
//! Online entries are never evicted; if they alone fill the table a new id is not tracked. Inputs
//! only come from keyed or verified ids, so the bound is a safety net. Liveness runs on the
//! monotonic clock, so a stepped wall clock neither keeps silent devices online nor refuses
//! heartbeats. Called from the signal server's UDP loop: one mutex, no I/O.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use crate::clock::SharedClock;

/// hbbs `REG_TIMEOUT`, the window OnlineRequest and punch holes use.
pub const SIGNAL_TTL: Duration = Duration::from_secs(30);
/// Three idle heartbeat periods (15 s) of the client.
pub const HEARTBEAT_TTL: Duration = Duration::from_secs(45);
pub const DEFAULT_CAPACITY: usize = 100_000;
pub const MAX_CONNS: usize = 64;
pub const SWEEP_INTERVAL: Duration = Duration::from_secs(1);
const FULL_WARN_INTERVAL_MS: i64 = 60_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DevicePresence {
    pub online: bool,
    /// `None` if only the identity is cached.
    pub last_seen_ms: Option<i64>,
    pub ip: Option<IpAddr>,
    /// Empty once the heartbeat expired, even if the signal registration keeps the device online.
    pub conns: Vec<i32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiveCounts {
    pub devices: usize,
    pub connections: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    pub rustdesk_id: String,
    pub last_seen_ms: i64,
    pub ip: Option<IpAddr>,
}

/// Both fixed for the row's lifetime; `has_sysinfo` only ever turns true.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Identity {
    pub device_id: i64,
    pub uuid: Vec<u8>,
    pub has_sysinfo: bool,
}

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
    last_sweep_ms: Option<i64>,
    last_full_warn_ms: Option<i64>,
    last_precision_warn_ms: Option<i64>,
    forgets: u64,
}

#[derive(Default)]
struct Entry {
    signal_at: Option<i64>,
    heartbeat_at: Option<i64>,
    seen_wall: Option<i64>,
    ip: Option<IpAddr>,
    conns: Vec<i32>,
    unflushed: bool,
    identity: Option<Identity>,
}

impl Entry {
    fn last_seen_ms(&self) -> Option<i64> {
        self.seen_wall
    }

    fn last_contact(&self) -> Option<i64> {
        self.signal_at.max(self.heartbeat_at)
    }

    fn is_online(&self, now: i64) -> bool {
        within(self.signal_at, SIGNAL_TTL, now) || self.heartbeat_is_fresh(now)
    }

    fn heartbeat_is_fresh(&self, now: i64) -> bool {
        within(self.heartbeat_at, HEARTBEAT_TTL, now)
    }

    fn live_conns(&self, now: i64) -> &[i32] {
        if self.heartbeat_is_fresh(now) {
            &self.conns
        } else {
            &[]
        }
    }

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

    pub fn signal_online(&self, rustdesk_id: &str, ip: IpAddr) {
        let wall = self.wall();
        self.update(rustdesk_id, |entry, now| {
            entry.signal_at = Some(now);
            entry.seen_wall = Some(wall);
            entry.ip = Some(ip);
            entry.unflushed = true;
        });
    }

    pub fn signal_offline(&self, rustdesk_id: &str) {
        if let Some(entry) = self.lock().entries.get_mut(rustdesk_id) {
            entry.signal_at = None;
        }
    }

    pub fn heartbeat(&self, rustdesk_id: &str, ip: IpAddr, conns: &[i32]) {
        self.try_heartbeat(rustdesk_id, ip, conns, Duration::ZERO);
    }

    /// An id the table has no room for is never refused.
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

    pub fn online_count(&self) -> usize {
        let now = self.now();
        let state = self.lock();
        state.entries.values().filter(|e| e.is_online(now)).count()
    }

    pub fn live(&self) -> LiveCounts {
        let now = self.now();
        let state = self.lock();
        let online = state.entries.values().filter(|e| e.is_online(now));
        online.fold(LiveCounts::default(), |counts, entry| LiveCounts {
            devices: counts.devices + 1,
            connections: counts.connections + entry.live_conns(now).len(),
        })
    }

    pub fn len(&self) -> usize {
        self.lock().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// A caller whose write fails reports the result again later; nothing is retried here.
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

    pub fn forget(&self, rustdesk_id: &str) {
        let mut state = self.lock();
        state.entries.remove(rustdesk_id);
        state.forgets += 1;
    }

    pub(crate) fn identity(&self, rustdesk_id: &str) -> Option<Identity> {
        let state = self.lock();
        state.entries.get(rustdesk_id)?.identity.clone()
    }

    pub(crate) fn identity_epoch(&self) -> u64 {
        self.lock().forgets
    }

    /// Not after a [`forget`](Self::forget) since `epoch`: the row read may be gone.
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

    fn now(&self) -> i64 {
        self.inner.clock.elapsed_ms()
    }

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

fn within(at: Option<i64>, ttl: Duration, now: i64) -> bool {
    at.is_some_and(|t| now - t < millis(ttl))
}

#[cfg(test)]
mod tests;
