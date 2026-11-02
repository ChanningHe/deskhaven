// Modified by deskhaven; see crates/deskhaven-signal/CHANGES.md (AGPL-3.0 §5a).
use crate::common::*;
use crate::store::{PeerStore, PeerUpsert, StoreError};
use crate::tasks::Tasks;
use hbb_common::{
    bytes::Bytes,
    log,
    tokio::{
        self,
        sync::{
            Mutex, RwLock,
            mpsc::{self, error::TrySendError},
        },
        time::{Duration, timeout},
    },
};
use std::{
    collections::HashMap,
    collections::HashSet,
    net::SocketAddr,
    sync::Arc,
    time::{Instant, SystemTime},
};
use tokio_util::sync::CancellationToken;

// deskhaven: crate-visible for `prune_ip_blocks` (rendezvous_server.rs).
pub(crate) type IpBlockMap = HashMap<String, ((u32, Instant), (HashSet<String>, Instant))>;
#[allow(dead_code)] // only used by the unused USER_STATUS static (upstream)
type UserStatusMap = HashMap<Vec<u8>, Arc<(Option<Vec<u8>>, bool)>>;
lazy_static::lazy_static! {
    pub(crate) static ref IP_BLOCKER: Mutex<IpBlockMap> = Default::default();
    pub(crate) static ref USER_STATUS: RwLock<UserStatusMap> = Default::default();
}
// deskhaven: IP_CHANGES, IpChangesMap, IP_CHANGE_DUR and IP_CHANGE_DUR_X2 removed: the map was
// only read by the removed console command `ic`, so it was write-only state.
pub const DAY_SECONDS: u64 = 3600 * 24;
pub const IP_BLOCK_DUR: u64 = 60;
// Same bound as the Pro server: a longer id is not a normal RustDesk id or hostname,
// and every peer entry and punch-request record keeps its own copy.
pub const MAX_PEER_ID_LEN: usize = 253;

// deskhaven: bounds of the store writer (see `PeerMap`). The queue absorbs bursts of
// registrations (rate limits: 3 RegisterPk per id per 6 s; per IP 30 per minute and 300 ids
// a day); when it is full the registration is refused with SERVER_ERROR and the client retries.
const WRITE_QUEUE: usize = 1024;
// One store call; a slower one counts as failed (the store's own timeouts should fire first).
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
// Flushing the queue when the server stops; what is left after it is lost (and logged).
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

// deskhaven: no serde any more; the info JSON column of upstream's `peer` table is gone
// (the store keeps the IP itself).
#[derive(Debug, Default, Clone)]
pub(crate) struct PeerInfo {
    pub(crate) ip: String,
}

pub(crate) struct Peer {
    pub(crate) socket_addr: SocketAddr,
    pub(crate) last_reg_time: Instant,
    // deskhaven: `guid` removed: it only told insert from update, which `upsert_pk` decides.
    pub(crate) uuid: Bytes,
    pub(crate) pk: Bytes,
    // pub(crate) user: Option<Vec<u8>>,
    pub(crate) info: PeerInfo,
    // pub(crate) disabled: bool,
    pub(crate) reg_pk: (u32, Instant), // how often register_pk
}

impl Default for Peer {
    fn default() -> Self {
        Self {
            socket_addr: "0.0.0.0:0".parse().unwrap(),
            last_reg_time: get_expired_time(),
            uuid: Bytes::new(),
            pk: Bytes::new(),
            info: Default::default(),
            // user: None,
            // disabled: false,
            reg_pk: (0, get_expired_time()),
        }
    }
}

pub(crate) type LockPeer = Arc<RwLock<Peer>>;
type PeerCache = Arc<RwLock<HashMap<String, LockPeer>>>;

/// deskhaven: an accepted registration on its way to the store.
struct PeerWrite {
    peer: LockPeer,
    rec: PeerUpsert,
}

// deskhaven: the in-memory map is a cache in front of the injected `PeerStore` (upstream: the
// SQLite `database.rs`). Lookups hit the store only for ids not in memory. Accepted
// registrations update memory at once and are persisted by one writer task, fed through a
// bounded queue, so the UDP loop never waits for a store write; a single writer keeps the
// writes in acceptance order, so the store ends with the same state as memory.
#[derive(Clone)]
pub(crate) struct PeerMap {
    map: PeerCache,
    store: Arc<dyn PeerStore>,
    writes: mpsc::Sender<PeerWrite>,
}

impl PeerMap {
    // deskhaven: upstream resolved DB_URL and opened SQLite here. Starts the store writer on
    // `tasks`; it flushes what is queued when the server stops.
    pub(crate) fn new(store: Arc<dyn PeerStore>, tasks: &Tasks) -> Self {
        let (writes, rx) = mpsc::channel(WRITE_QUEUE);
        let map = PeerCache::default();
        let writer = Writer {
            map: map.clone(),
            store: store.clone(),
        };
        tasks.spawn_draining(move |stop| writer.run(rx, stop));
        Self { map, store, writes }
    }

    // deskhaven: queues the store write instead of awaiting it. Fails, before touching the
    // peer, only when the queue is full or the writer has stopped; the caller then refuses
    // the registration. Upstream ignored the (awaited) result and always answered OK.
    #[inline]
    pub(crate) async fn update_pk(
        &mut self,
        id: String,
        peer: LockPeer,
        addr: SocketAddr,
        uuid: Bytes,
        pk: Bytes,
        ip: String,
    ) -> Result<(), TrySendError<()>> {
        log::info!("update_pk {} {:?} {:?} {:?}", id, addr, uuid, pk);
        let permit = self.writes.try_reserve().inspect_err(|err| {
            log::error!("peer store: registration of {id} not queued: {err}");
        })?;
        {
            let mut w = peer.write().await;
            w.socket_addr = addr;
            w.uuid = uuid.clone();
            w.pk = pk.clone();
            w.last_reg_time = Instant::now();
            w.info.ip = ip;
        }
        let rec = PeerUpsert {
            id,
            uuid: uuid.to_vec(),
            pk: pk.to_vec(),
            ip: addr.ip(),
            registered_at: SystemTime::now(),
        };
        permit.send(PeerWrite { peer, rec });
        Ok(())
    }

    // deskhaven: a store failure is logged and reads as "unknown id" (nothing is cached).
    #[inline]
    pub(crate) async fn get(&self, id: &str) -> Option<LockPeer> {
        self.load(id).await.unwrap_or_else(|err| {
            log::error!("peer store: lookup of {id} failed: {err:?}");
            None
        })
    }

    // deskhaven: fails if the store fails, instead of handing out a fresh entry that would let
    // any uuid claim a registered id (fail closed).
    #[inline]
    pub(crate) async fn get_or(&self, id: &str) -> Result<LockPeer, StoreError> {
        if let Some(p) = self.load(id).await? {
            return Ok(p);
        }
        Ok(self
            .map
            .write()
            .await
            .entry(id.to_owned())
            .or_default()
            .clone())
    }

    // deskhaven: upstream `get`, returning store errors. An entry another task added while
    // the store was read is kept (upstream overwrote it).
    async fn load(&self, id: &str) -> Result<Option<LockPeer>, StoreError> {
        if let Some(p) = self.get_in_memory(id).await {
            return Ok(Some(p));
        }
        let Some(v) = self.store.get(id).await? else {
            return Ok(None);
        };
        let peer = Peer {
            uuid: v.uuid.into(),
            pk: v.pk.into(),
            // user: v.user,
            info: PeerInfo {
                ip: v.ip.map(|ip| ip.to_string()).unwrap_or_default(),
            },
            // disabled: v.status == Some(0),
            ..Default::default()
        };
        let mut map = self.map.write().await;
        let entry = map
            .entry(id.to_owned())
            .or_insert_with(|| Arc::new(RwLock::new(peer)));
        Ok(Some(entry.clone()))
    }

    #[inline]
    pub(crate) async fn get_in_memory(&self, id: &str) -> Option<LockPeer> {
        self.map.read().await.get(id).cloned()
    }

    #[inline]
    #[allow(dead_code)] // unused upstream
    pub(crate) async fn is_in_memory(&self, id: &str) -> bool {
        self.map.read().await.contains_key(id)
    }
}

/// deskhaven: the store writer of one server instance.
struct Writer {
    map: PeerCache,
    store: Arc<dyn PeerStore>,
}

impl Writer {
    async fn run(self, mut rx: mpsc::Receiver<PeerWrite>, stop: CancellationToken) {
        loop {
            let next = tokio::select! {
                next = rx.recv() => next,
                () = stop.cancelled() => break,
            };
            let Some(write) = next else { return };
            self.persist(write).await;
        }
        // Registrations already answered OK still reach the store, within a bound.
        rx.close();
        let drain = async {
            while let Some(write) = rx.recv().await {
                self.persist(write).await;
            }
        };
        if timeout(DRAIN_TIMEOUT, drain).await.is_err() {
            log::error!("peer store: shutdown flush timed out; queued registrations are lost");
        }
    }

    /// On failure the peer is dropped from memory, so the store stays the truth: its next
    /// RegisterPeer is answered `request_pk`, and the RegisterPk that follows is checked
    /// against the store again (and its write retried).
    async fn persist(&self, PeerWrite { peer, rec }: PeerWrite) {
        let res = match timeout(WRITE_TIMEOUT, self.store.upsert_pk(&rec)).await {
            Ok(res) => res,
            Err(_) => Err(StoreError::backend("store write timed out")),
        };
        let Err(err) = res else { return };
        log::error!("peer store: registration of {} not saved: {err:?}", rec.id);
        let mut map = self.map.write().await;
        if map.get(&rec.id).is_some_and(|p| Arc::ptr_eq(p, &peer)) {
            map.remove(&rec.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{MemoryPeerStore, PeerRecord};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, Ordering};

    const ID: &str = "123456789";

    /// Fails every call while `failing` is set; otherwise delegates to a memory store.
    #[derive(Default)]
    struct FlakyStore {
        failing: AtomicBool,
        inner: MemoryPeerStore,
    }

    impl FlakyStore {
        fn check(&self) -> Result<(), StoreError> {
            if self.failing.load(Ordering::SeqCst) {
                return Err(StoreError::backend("injected failure"));
            }
            Ok(())
        }
    }

    #[async_trait]
    impl PeerStore for FlakyStore {
        async fn get(&self, id: &str) -> Result<Option<PeerRecord>, StoreError> {
            self.check()?;
            self.inner.get(id).await
        }

        async fn upsert_pk(&self, rec: &PeerUpsert) -> Result<(), StoreError> {
            self.check()?;
            self.inner.upsert_pk(rec).await
        }
    }

    fn addr() -> SocketAddr {
        "203.0.113.5:40000".parse().unwrap()
    }

    async fn register(pm: &mut PeerMap, uuid: &'static [u8]) -> Result<(), TrySendError<()>> {
        let peer = pm.get_or(ID).await.unwrap();
        let ip = addr().ip().to_string();
        let (uuid, pk) = (Bytes::from_static(uuid), Bytes::from_static(b"pk"));
        pm.update_pk(ID.to_owned(), peer, addr(), uuid, pk, ip)
            .await
    }

    #[tokio::test]
    async fn stop_flushes_queued_writes_to_the_store() {
        let store = Arc::new(MemoryPeerStore::new());
        let tasks = Tasks::new(CancellationToken::new());
        let mut pm = PeerMap::new(store.clone(), &tasks);

        register(&mut pm, b"uuid").await.unwrap();
        tasks.shutdown().await;

        let rec = store.get(ID).await.unwrap().unwrap();
        assert_eq!((rec.uuid, rec.ip), (b"uuid".to_vec(), Some(addr().ip())));
    }

    #[tokio::test]
    async fn store_lookup_failure_fails_closed_and_caches_nothing() {
        let store = Arc::new(FlakyStore::default());
        let tasks = Tasks::new(CancellationToken::new());
        let pm = PeerMap::new(store.clone(), &tasks);
        store.failing.store(true, Ordering::SeqCst);

        assert!(pm.get_or(ID).await.is_err());
        assert!(pm.get(ID).await.is_none());
        assert!(pm.get_in_memory(ID).await.is_none());
        tasks.shutdown().await;
    }

    #[tokio::test]
    async fn failed_write_evicts_the_peer_so_the_store_is_asked_again() {
        let store = Arc::new(FlakyStore::default());
        let tasks = Tasks::new(CancellationToken::new());
        let mut pm = PeerMap::new(store.clone(), &tasks);
        store.failing.store(true, Ordering::SeqCst);
        let peer = pm
            .map
            .write()
            .await
            .entry(ID.to_owned())
            .or_default()
            .clone();

        let ip = addr().ip().to_string();
        let (uuid, pk) = (Bytes::from_static(b"uuid"), Bytes::from_static(b"pk"));
        pm.update_pk(ID.to_owned(), peer, addr(), uuid, pk, ip)
            .await
            .unwrap();
        tasks.shutdown().await;

        assert!(pm.get_in_memory(ID).await.is_none(), "evicted");
    }

    #[tokio::test]
    async fn full_queue_refuses_before_touching_the_peer() {
        let store = Arc::new(MemoryPeerStore::new());
        let tasks = Tasks::new(CancellationToken::new());
        let mut pm = PeerMap::new(store, &tasks);
        // A queue nobody drains, full after one write.
        let (tx, _rx) = mpsc::channel(1);
        pm.writes = tx;
        register(&mut pm, b"first").await.unwrap();
        let peer = pm.get_or(ID).await.unwrap();
        let before = peer.read().await.uuid.clone();

        let res = register(&mut pm, b"second").await;

        assert!(matches!(res, Err(TrySendError::Full(()))));
        assert_eq!(peer.read().await.uuid, before);
        tasks.shutdown().await;
    }
}
