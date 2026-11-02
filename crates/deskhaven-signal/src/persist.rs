//! Persistence of the peer map (deskhaven code, not vendored): startup preload, coalesced
//! write-behind of accepted registrations, and an invalidation hook.
//!
//! - **Preload**: every stored registration is loaded, page by page (keyset on the id), before
//!   the listeners are bound, so afterwards a cache miss means "unknown id" and the UDP / TCP
//!   paths never wait for the store.
//! - **Write-behind, coalesced per peer**: an accepted RegisterPk updates memory, bumps the
//!   peer's [`SaveState::generation`] and is answered OK; the peer's *id* is queued for one
//!   writer task. An id is queued at most once, and the writer reads the peer's current state
//!   when it writes, so repeated registrations collapse into one write of the latest state
//!   and the queue never holds more entries than there are peers.
//! - **Store outage**: memory stays authoritative and devices stay online (upstream behaved
//!   the same, it ignored write errors). A failed write re-queues the id; the writer retries it
//!   itself, nothing is asked of the client. After [`BREAKER_THRESHOLD`] consecutive failures a
//!   circuit breaker stops all attempts for a cool-down (doubling up to [`COOL_DOWN_MAX`]), then
//!   probes with one write; so an outage costs a few attempts per minute, not a stall per
//!   write. The queue's bound ([`WRITE_QUEUE`]) only refuses ids that are not bound yet
//!   (SERVER_ERROR, the client retries); a bound peer is always accepted, its id coalesced.
//! - **UuidMismatch** from the store (the id is bound to another device there, e.g. changed
//!   out of band): the store wins; its record is re-read into the cached peer in place.
//! - **Stop**: within one deadline, each queued write gets a single attempt (no breaker, no
//!   backoff); what is left unsaved is counted and logged.
//!
//! Never-online `devices` rows are not cleaned up here; retention comes with admission (M5) and
//! the device lifecycle (M2-20).

use crate::peer::{LockPeer, Peer, PeerCache, PeerInfo};
use crate::store::{PeerRecord, PeerStore, PeerUpsert, StoreError};
use crate::tasks::Tasks;
use hbb_common::{
    log,
    tokio::{
        self,
        sync::{Notify, RwLock},
        time::{Duration, Instant, sleep, timeout},
    },
};
use std::{
    collections::{HashSet, VecDeque},
    net::IpAddr,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::SystemTime,
};
use tokio_util::sync::CancellationToken;

/// Queued ids beyond which ids that are not bound yet are refused. Registrations are
/// rate-limited (3 RegisterPk per id per 6 s; per IP 30 per minute and 300 ids a day).
pub(crate) const WRITE_QUEUE: usize = 1024;
/// One store call; a slower one counts as a failure.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// Consecutive failed writes that open the circuit breaker.
pub(crate) const BREAKER_THRESHOLD: u32 = 3;
/// First cool-down of an open breaker; doubled on every failed probe.
pub(crate) const COOL_DOWN_MIN: Duration = Duration::from_secs(1);
/// Longest cool-down.
pub(crate) const COOL_DOWN_MAX: Duration = Duration::from_secs(30);
/// Everything the writer still does after the stop, in-flight write included.
pub(crate) const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
/// Registrations per preload query.
pub(crate) const PRELOAD_PAGE: usize = 1000;

/// Persistence bookkeeping of one cached peer.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SaveState {
    /// Bumped on every accepted registration.
    pub(crate) generation: u64,
    /// The generation last written to (or read from) the store.
    pub(crate) saved: u64,
    /// When the last registration was accepted.
    pub(crate) registered_at: SystemTime,
}

impl Default for SaveState {
    fn default() -> Self {
        Self {
            generation: 0,
            saved: 0,
            registered_at: SystemTime::UNIX_EPOCH,
        }
    }
}

impl SaveState {
    pub(crate) fn is_saved(&self) -> bool {
        self.saved == self.generation
    }

    pub(crate) fn accepted(&mut self) {
        self.generation += 1;
        self.registered_at = SystemTime::now();
    }
}

/// Refused: the write queue is full and the id is not bound yet.
#[derive(Debug)]
pub(crate) struct QueueFull;

/// Ids waiting for the writer, each at most once.
#[derive(Default)]
struct Pending {
    order: VecDeque<String>,
    queued: HashSet<String>,
}

impl Pending {
    fn push(&mut self, id: &str) {
        if self.queued.insert(id.to_owned()) {
            self.order.push_back(id.to_owned());
        }
    }

    fn pop(&mut self) -> Option<String> {
        let id = self.order.pop_front()?;
        self.queued.remove(&id);
        Some(id)
    }
}

/// The store side of one server's peer map. Cheap to clone.
#[derive(Clone)]
pub(crate) struct Persist {
    store: Arc<dyn PeerStore>,
    map: PeerCache,
    pending: Arc<Mutex<Pending>>,
    wake: Arc<Notify>,
    cap: usize,
    /// Writes left unsaved when the writer stopped.
    lost: Arc<AtomicUsize>,
}

impl Persist {
    /// Starts the writer task on `tasks`; `cap` bounds the queue for unbound ids.
    pub(crate) fn start(
        store: Arc<dyn PeerStore>,
        map: PeerCache,
        tasks: &Tasks,
        cap: usize,
    ) -> Self {
        let persist = Self {
            store,
            map,
            pending: Arc::default(),
            wake: Arc::default(),
            cap,
            lost: Arc::default(),
        };
        let writer = Writer {
            persist: persist.clone(),
            in_flight: AtomicBool::new(false),
            failed: AtomicUsize::new(0),
        };
        tasks.spawn_draining(move |stop| writer.run(stop));
        persist
    }

    fn pending(&self) -> MutexGuard<'_, Pending> {
        // Every update is a single push / pop, consistent even after a panic.
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queues `id` for the writer. Call with the peer's write lock held and change the peer
    /// only on `Ok`: the writer reads the peer after taking its lock, so it sees the change.
    /// An id already queued is not queued again. Only an unbound id (`is_new`) is refused, and
    /// only when the queue is full.
    pub(crate) fn enqueue(&self, id: &str, is_new: bool) -> Result<(), QueueFull> {
        let mut pending = self.pending();
        if pending.queued.contains(id) {
            return Ok(());
        }
        if is_new && pending.order.len() >= self.cap {
            log::error!("peer store: registration of new id {id} refused, write queue full");
            return Err(QueueFull);
        }
        pending.push(id);
        drop(pending);
        self.wake.notify_one();
        Ok(())
    }

    fn pop(&self) -> Option<String> {
        self.pending().pop()
    }

    fn requeue(&self, id: &str) {
        self.pending().push(id);
    }

    pub(crate) fn queued_len(&self) -> usize {
        self.pending().order.len()
    }

    #[cfg(test)]
    pub(crate) fn lost_at_stop(&self) -> usize {
        self.lost.load(Ordering::SeqCst)
    }

    /// Loads every stored registration into the map, a page at a time; fails if the store
    /// fails or the whole load takes longer than `limit`.
    pub(crate) async fn preload(&self, limit: Duration) -> Result<usize, StoreError> {
        let started = Instant::now();
        let count = timeout(limit, self.load_pages())
            .await
            .map_err(|_| StoreError::backend(format!("loading timed out after {limit:?}")))??;
        log::info!(
            "peer store: preloaded {count} registrations in {:?}",
            started.elapsed()
        );
        Ok(count)
    }

    async fn load_pages(&self) -> Result<usize, StoreError> {
        let mut after: Option<String> = None;
        let mut count = 0;
        loop {
            let page = self.store.load_page(after.as_deref(), PRELOAD_PAGE).await?;
            let full = page.len() >= PRELOAD_PAGE;
            count += page.len();
            after = page.last().map(|rec| rec.id.clone());
            let mut map = self.map.write().await;
            for rec in page {
                let id = rec.id.clone();
                map.insert(id, Arc::new(RwLock::new(peer_from(rec))));
            }
            if !full {
                return Ok(count);
            }
        }
    }

    /// Invalidation hook for registrations changed out of band (admin edits / deletes, M2;
    /// also used on UuidMismatch): re-reads `id` from the store. A stored record replaces the
    /// cached keys in place (address, liveness and rate-limit counters kept, the peer counts as
    /// saved); a missing one is forgotten.
    pub(crate) async fn reload(&self, id: &str) -> Result<(), StoreError> {
        let stored = self.store.get(id).await?;
        let cached = self.map.read().await.get(id).cloned();
        match (stored, cached) {
            (None, _) => {
                self.map.write().await.remove(id);
            }
            (Some(rec), None) => {
                let fresh = Arc::new(RwLock::new(peer_from(rec)));
                self.map.write().await.entry(id.to_owned()).or_insert(fresh);
            }
            (Some(rec), Some(peer)) => {
                let mut p = peer.write().await;
                p.uuid = rec.uuid.into();
                p.pk = rec.pk.into();
                p.info.ip = rec.ip.map(|ip| ip.to_string()).unwrap_or_default();
                p.save.saved = p.save.generation;
            }
        }
        Ok(())
    }
}

/// A stored registration as an offline cached peer.
fn peer_from(rec: PeerRecord) -> Peer {
    Peer {
        uuid: rec.uuid.into(),
        pk: rec.pk.into(),
        info: PeerInfo {
            ip: rec.ip.map(|ip| ip.to_string()).unwrap_or_default(),
        },
        ..Default::default()
    }
}

/// Result of one write.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// The store answered (saved, or rebound on UuidMismatch).
    Stored,
    /// Nothing to write (already saved, or the peer is gone).
    Skipped,
    /// The store failed or timed out; the write is still owed.
    Failed,
}

/// Opens after [`BREAKER_THRESHOLD`] consecutive failures and stays open for a cool-down that
/// doubles on every failed probe.
struct Breaker {
    failures: u32,
    open_until: Option<Instant>,
    cool_down: Duration,
}

impl Default for Breaker {
    fn default() -> Self {
        Self {
            failures: 0,
            open_until: None,
            cool_down: COOL_DOWN_MIN,
        }
    }
}

impl Breaker {
    /// How long writes must still wait.
    fn wait(&self) -> Option<Duration> {
        let left = self.open_until?.saturating_duration_since(Instant::now());
        (!left.is_zero()).then_some(left)
    }

    fn failure(&mut self, pending: usize) {
        self.failures += 1;
        if self.failures < BREAKER_THRESHOLD {
            return;
        }
        log::error!(
            "peer store: unavailable, pausing writes for {:?} ({pending} registrations unsaved; \
             served from memory)",
            self.cool_down
        );
        self.open_until = Some(Instant::now() + self.cool_down);
        self.cool_down = (self.cool_down * 2).min(COOL_DOWN_MAX);
    }

    fn success(&mut self) {
        if self.failures >= BREAKER_THRESHOLD {
            log::info!("peer store: available again, resuming writes");
        }
        *self = Self::default();
    }
}

struct Writer {
    persist: Persist,
    in_flight: AtomicBool,
    /// Writes that failed during the stop.
    failed: AtomicUsize,
}

impl Writer {
    async fn run(self, stop: CancellationToken) {
        let work = async {
            self.serve(&stop).await;
            self.drain().await;
        };
        let deadline = async {
            stop.cancelled().await;
            sleep(DRAIN_TIMEOUT).await;
        };
        let timed_out = tokio::select! {
            () = work => false,
            () = deadline => true,
        };
        let mut unsaved = self.failed.load(Ordering::SeqCst);
        if timed_out {
            unsaved +=
                self.persist.queued_len() + usize::from(self.in_flight.load(Ordering::SeqCst));
        }
        self.persist.lost.store(unsaved, Ordering::SeqCst);
        if unsaved > 0 {
            log::error!(
                "peer store: stopped with {unsaved} registrations unsaved{} (the clients \
                 re-send them)",
                if timed_out {
                    " at the flush deadline"
                } else {
                    ""
                }
            );
        }
    }

    /// Writes queued ids until the stop, behind the circuit breaker.
    async fn serve(&self, stop: &CancellationToken) {
        let mut breaker = Breaker::default();
        loop {
            if let Some(wait) = breaker.wait() {
                tokio::select! {
                    () = sleep(wait) => {}
                    () = stop.cancelled() => return,
                }
            }
            let id = loop {
                if stop.is_cancelled() {
                    return;
                }
                if let Some(id) = self.persist.pop() {
                    break id;
                }
                tokio::select! {
                    () = self.persist.wake.notified() => {}
                    () = stop.cancelled() => return,
                }
            };
            match self.write(&id).await {
                Outcome::Stored => breaker.success(),
                Outcome::Skipped => {}
                Outcome::Failed => {
                    self.persist.requeue(&id);
                    breaker.failure(self.persist.queued_len());
                }
            }
        }
    }

    /// After the stop: one attempt per queued id, no breaker.
    async fn drain(&self) {
        while let Some(id) = self.persist.pop() {
            if self.write(&id).await == Outcome::Failed {
                self.failed.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    async fn write(&self, id: &str) -> Outcome {
        self.in_flight.store(true, Ordering::SeqCst);
        let outcome = self.write_current(id).await;
        self.in_flight.store(false, Ordering::SeqCst);
        outcome
    }

    /// Writes the peer's current state, if not saved yet.
    async fn write_current(&self, id: &str) -> Outcome {
        let Some(peer) = self.persist.map.read().await.get(id).cloned() else {
            return Outcome::Skipped;
        };
        let Some((rec, generation)) = snapshot(id, &peer).await else {
            return Outcome::Skipped;
        };
        let res = match timeout(WRITE_TIMEOUT, self.persist.store.upsert_pk(&rec)).await {
            Ok(res) => res,
            Err(_) => Err(StoreError::backend("store write timed out")),
        };
        match res {
            Ok(()) => {
                let mut p = peer.write().await;
                p.save.saved = p.save.saved.max(generation);
                Outcome::Stored
            }
            Err(StoreError::UuidMismatch { .. }) => {
                log::error!(
                    "peer store: {id} is bound to another uuid in the store; loading that binding"
                );
                match self.persist.reload(id).await {
                    Ok(()) => Outcome::Stored,
                    Err(err) => {
                        log::warn!("peer store: reloading {id} failed: {err:?}");
                        Outcome::Failed
                    }
                }
            }
            Err(err) => {
                log::warn!("peer store: write of {id} failed: {err:?}");
                Outcome::Failed
            }
        }
    }
}

/// The upsert for the peer's current state and its generation, unless there is nothing to
/// write.
async fn snapshot(id: &str, peer: &LockPeer) -> Option<(PeerUpsert, u64)> {
    let p = peer.read().await;
    if p.uuid.is_empty() || p.save.is_saved() {
        return None;
    }
    let ip = p
        .info
        .ip
        .parse::<IpAddr>()
        .unwrap_or_else(|_| p.socket_addr.ip());
    let rec = PeerUpsert {
        id: id.to_owned(),
        uuid: p.uuid.to_vec(),
        pk: p.pk.to_vec(),
        ip,
        registered_at: p.save.registered_at,
    };
    Some((rec, p.save.generation))
}

#[cfg(test)]
mod tests;
