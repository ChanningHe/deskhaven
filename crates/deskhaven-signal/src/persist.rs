//! Persistence of the peer map (deskhaven code, not vendored): startup preload, ordered
//! write-behind of accepted registrations, and an invalidation hook.
//!
//! - **Preload**: every stored registration is loaded before the listeners are bound, so
//!   afterwards a cache miss means "unknown id" and the UDP / TCP paths never wait for the
//!   store (no lookup can stall the loop, no request can drain the connection pool).
//! - **Write-behind**: an accepted RegisterPk is answered OK once memory is updated; the store
//!   write goes through a bounded queue to one writer task. One writer keeps the writes in
//!   acceptance order, so the store ends with the state memory has. A full queue refuses the
//!   registration before memory changes (the caller answers SERVER_ERROR, the client retries).
//! - **Failures are retried, never silently dropped**: a backend error keeps the accepted
//!   entry in memory and retries with backoff; after the last retry the peer is marked
//!   `dirty`, so its next RegisterPeer asks for its pk again and the following RegisterPk
//!   re-queues the write. The store answering UuidMismatch (it binds the id to another device,
//!   e.g. changed out of band) is the one case where memory yields: the entry is evicted.
//! - **Stop**: the writer finishes what is queued, including the write in flight, within one
//!   deadline; whatever is left then is cancelled and logged as lost.

use crate::peer::{LockPeer, Peer, PeerCache, PeerInfo};
use crate::store::{PeerRecord, PeerStore, PeerUpsert, StoreError};
use crate::tasks::Tasks;
use hbb_common::{
    log,
    tokio::{
        self,
        sync::{
            RwLock,
            mpsc::{self, Permit, error::TrySendError},
        },
        time::{Duration, sleep, timeout},
    },
};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Queued writes. Registrations are rate-limited (3 RegisterPk per id per 6 s; per IP 30 per
/// minute and 300 ids a day), so a full queue means the store is not keeping up.
const WRITE_QUEUE: usize = 1024;
/// One store call; a slower one counts as a backend failure.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// Waits between the attempts of one write (so 1 + 4 attempts).
const RETRY_BACKOFF: [Duration; 4] = [
    Duration::from_millis(250),
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
];
/// Everything the writer still does after the stop, in-flight write included.
pub(crate) const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// An accepted registration on its way to the store.
pub(crate) struct PeerWrite {
    pub(crate) peer: LockPeer,
    pub(crate) rec: PeerUpsert,
}

/// The store side of one server's peer map. Cheap to clone.
#[derive(Clone)]
pub(crate) struct Persist {
    store: Arc<dyn PeerStore>,
    map: PeerCache,
    writes: mpsc::Sender<PeerWrite>,
}

impl Persist {
    /// Starts the writer task on `tasks`.
    pub(crate) fn start(store: Arc<dyn PeerStore>, map: PeerCache, tasks: &Tasks) -> Self {
        let (writes, rx) = mpsc::channel(WRITE_QUEUE);
        let writer = Writer {
            store: store.clone(),
            map: map.clone(),
        };
        tasks.spawn_draining(move |stop| writer.run(rx, stop));
        Self { store, map, writes }
    }

    /// A queue slot for one write, or why there is none (queue full, writer stopped).
    pub(crate) fn reserve(&self) -> Result<Permit<'_, PeerWrite>, TrySendError<()>> {
        self.writes.try_reserve().inspect_err(|err| {
            log::error!("peer store: registration refused, write queue unavailable: {err}");
        })
    }

    /// Loads every stored registration into the map; fails if the store fails or takes longer
    /// than `limit`.
    pub(crate) async fn preload(&self, limit: Duration) -> Result<usize, StoreError> {
        let records = timeout(limit, self.store.load_all())
            .await
            .map_err(|_| StoreError::backend(format!("loading timed out after {limit:?}")))??;
        let count = records.len();
        let mut map = self.map.write().await;
        for rec in records {
            let id = rec.id.clone();
            map.insert(id, Arc::new(RwLock::new(peer_from(rec))));
        }
        Ok(count)
    }

    /// Invalidation hook for registrations changed out of band (admin edits / deletes, M2):
    /// re-reads `id` from the store; a stored one replaces the cached keys (address and
    /// liveness kept), a missing one is forgotten. Writes still queued for `id` are applied
    /// afterwards and win.
    #[allow(dead_code)] // not wired yet
    pub(crate) async fn reload(&self, id: &str) -> Result<(), StoreError> {
        let stored = self.store.get(id).await?;
        let cached = self.map.read().await.get(id).cloned();
        let Some(rec) = stored else {
            self.map.write().await.remove(id);
            return Ok(());
        };
        let mut fresh = peer_from(rec);
        if let Some(cached) = cached {
            let cached = cached.read().await;
            fresh.socket_addr = cached.socket_addr;
            fresh.last_reg_time = cached.last_reg_time;
        }
        let fresh = Arc::new(RwLock::new(fresh));
        self.map.write().await.insert(id.to_owned(), fresh);
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

/// What the writer loop does next.
enum Next {
    Write(PeerWrite),
    Stop,
    Done,
}

struct Writer {
    store: Arc<dyn PeerStore>,
    map: PeerCache,
}

impl Writer {
    async fn run(self, mut rx: mpsc::Receiver<PeerWrite>, stop: CancellationToken) {
        let work = async {
            let mut stopping = false;
            loop {
                let next = tokio::select! {
                    next = rx.recv() => next.map_or(Next::Done, Next::Write),
                    () = stop.cancelled(), if !stopping => Next::Stop,
                };
                match next {
                    Next::Write(write) => self.persist(write).await,
                    // No new writes; the queued ones are still received.
                    Next::Stop => {
                        rx.close();
                        stopping = true;
                    }
                    Next::Done => return,
                }
            }
        };
        let deadline = async {
            stop.cancelled().await;
            sleep(DRAIN_TIMEOUT).await;
        };
        tokio::select! {
            () = work => {}
            () = deadline => log::error!(
                "peer store: shutdown flush deadline ({DRAIN_TIMEOUT:?}) reached; \
                 unsaved registrations are lost (they are re-sent by the clients)"
            ),
        }
    }

    async fn persist(&self, PeerWrite { peer, rec }: PeerWrite) {
        let mut retries = RETRY_BACKOFF.iter();
        loop {
            let res = match timeout(WRITE_TIMEOUT, self.store.upsert_pk(&rec)).await {
                Ok(res) => res,
                Err(_) => Err(StoreError::backend("store write timed out")),
            };
            let err = match res {
                Ok(()) => {
                    let mut p = peer.write().await;
                    if matches(&p, &rec) {
                        p.dirty = false;
                    }
                    return;
                }
                Err(StoreError::UuidMismatch { .. }) => {
                    log::error!(
                        "peer store: {} is bound to another uuid in the store; dropping the \
                         cached registration",
                        rec.id
                    );
                    self.evict(&rec.id, &peer).await;
                    return;
                }
                Err(err) => err,
            };
            if !matches(&*peer.read().await, &rec) {
                // A newer registration of this peer was accepted; its write is queued behind
                // this one and carries the current state.
                log::warn!(
                    "peer store: write of {} failed, superseded: {err:?}",
                    rec.id
                );
                return;
            }
            let Some(delay) = retries.next() else {
                log::error!(
                    "peer store: registration of {} NOT SAVED after {} attempts; kept in memory, \
                     re-requested from the client: {err:?}",
                    rec.id,
                    RETRY_BACKOFF.len() + 1
                );
                peer.write().await.dirty = true;
                return;
            };
            log::warn!(
                "peer store: write of {} failed, retrying in {delay:?}: {err:?}",
                rec.id
            );
            sleep(*delay).await;
        }
    }

    async fn evict(&self, id: &str, peer: &LockPeer) {
        let mut map = self.map.write().await;
        if map.get(id).is_some_and(|p| Arc::ptr_eq(p, peer)) {
            map.remove(id);
        }
    }
}

/// Whether `rec` still describes the cached peer (no newer registration accepted).
fn matches(peer: &Peer, rec: &PeerUpsert) -> bool {
    peer.uuid == rec.uuid && peer.pk == rec.pk && peer.info.ip == rec.ip.to_string()
}

#[cfg(test)]
mod tests;
