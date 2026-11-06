// Modified by deskhaven; see crates/deskhaven-signal/CHANGES.md (AGPL-3.0 §5a).
use crate::common::*;
use crate::ip_blocker::IpBlocker;
use crate::log_limit::Redacted;
use crate::persist::{Persist, QueueFull, SaveState, WRITE_QUEUE};
use crate::store::{PeerStore, StoreError};
use crate::tasks::Tasks;
use hbb_common::{
    bytes::Bytes,
    log,
    tokio::{
        sync::{Mutex, RwLock},
        time::Duration,
    },
};
use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Instant};

// deskhaven (M4-05): the map behind IP_BLOCKER is `ip_blocker::IpBlocker` (sources keyed by
// IPv4 address / IPv6 /64, id hashes, eviction instead of a lockout when full).
#[allow(dead_code)] // only used by the unused USER_STATUS static (upstream)
type UserStatusMap = HashMap<Vec<u8>, Arc<(Option<Vec<u8>>, bool)>>;
lazy_static::lazy_static! {
    pub(crate) static ref IP_BLOCKER: Mutex<IpBlocker> = Default::default();
    pub(crate) static ref USER_STATUS: RwLock<UserStatusMap> = Default::default();
}
// deskhaven: IP_CHANGES, IpChangesMap, IP_CHANGE_DUR and IP_CHANGE_DUR_X2 removed: the map was
// only read by the removed console command `ic`, so it was write-only state.
pub const DAY_SECONDS: u64 = 3600 * 24;
pub const IP_BLOCK_DUR: u64 = 60;
// Same bound as the Pro server: a longer id is not a normal RustDesk id or hostname,
// and every peer entry and punch-request record keeps its own copy.
pub const MAX_PEER_ID_LEN: usize = 253;

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
    // deskhaven: write-behind bookkeeping (persist.rs).
    pub(crate) save: SaveState,
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
            save: SaveState::default(),
        }
    }
}

pub(crate) type LockPeer = Arc<RwLock<Peer>>;
pub(crate) type PeerCache = Arc<RwLock<HashMap<String, LockPeer>>>;

// deskhaven: memory is authoritative while serving (upstream: a cache in front of SQLite).
// It is filled from the injected `PeerStore` before the listeners start (`preload`), so
// lookups never touch the store; accepted registrations are written behind (persist.rs).
#[derive(Clone)]
pub(crate) struct PeerMap {
    map: PeerCache,
    persist: Persist,
}

impl PeerMap {
    // deskhaven: upstream resolved DB_URL and opened SQLite here.
    pub(crate) fn new(store: Arc<dyn PeerStore>, tasks: &Tasks) -> Self {
        Self::with_write_queue(store, tasks, WRITE_QUEUE)
    }

    // deskhaven: `cap` bounds the write queue for unbound ids (persist.rs).
    pub(crate) fn with_write_queue(store: Arc<dyn PeerStore>, tasks: &Tasks, cap: usize) -> Self {
        let map = PeerCache::default();
        let persist = Persist::start(store, map.clone(), tasks, cap);
        Self { map, persist }
    }

    // deskhaven: loads every stored registration (startup, before serving).
    pub(crate) async fn preload(&self, limit: Duration) -> Result<usize, StoreError> {
        self.persist.preload(limit).await
    }

    // deskhaven: the store write is queued (coalesced per id), not awaited. Fails, before
    // touching the peer, only for an id that is not bound yet when the queue is full; the
    // caller then refuses the registration. Upstream awaited the database and ignored the
    // result.
    // deskhaven (M1-07 review, H1): a refused id also loses the default entry `get_or` made
    // for it, so no peer without a key stays behind (it answered punch holes as OFFLINE and
    // had its empty pk signed). Removed under the peer's lock (lock order peer -> map; nothing
    // waits for a peer while holding the map), and only if the entry is still this peer.
    #[inline]
    pub(crate) async fn update_pk(
        &mut self,
        id: String,
        peer: LockPeer,
        addr: SocketAddr,
        uuid: Bytes,
        pk: Bytes,
        ip: String,
    ) -> Result<(), QueueFull> {
        // deskhaven (M2-12 review): the uuid is never logged (proof of identity for the
        // device endpoints), only its length.
        log::info!(
            "update_pk {} {:?} uuid {} {:?}",
            id,
            addr,
            Redacted(&uuid),
            pk
        );
        let mut w = peer.write().await;
        // Queued under the peer's lock: the writer reads the peer after this change.
        if let Err(full) = self.persist.enqueue(&id, w.uuid.is_empty()) {
            let mut map = self.map.write().await;
            if w.uuid.is_empty() && map.get(&id).is_some_and(|p| Arc::ptr_eq(p, &peer)) {
                map.remove(&id);
            }
            return Err(full);
        }
        w.socket_addr = addr;
        w.uuid = uuid;
        w.pk = pk;
        w.last_reg_time = Instant::now();
        w.info.ip = ip;
        w.save.accepted();
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn persist(&self) -> &Persist {
        &self.persist
    }

    // deskhaven: memory only; after the preload a miss means the id is not registered.
    #[inline]
    pub(crate) async fn get(&self, id: &str) -> Option<LockPeer> {
        self.get_in_memory(id).await
    }

    #[inline]
    pub(crate) async fn get_or(&self, id: &str) -> LockPeer {
        if let Some(p) = self.get(id).await {
            return p;
        }
        let mut w = self.map.write().await;
        if let Some(p) = w.get(id) {
            return p.clone();
        }
        let tmp = LockPeer::default();
        w.insert(id.to_owned(), tmp.clone());
        tmp
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

    // deskhaven: invalidation hook for out-of-band registry changes: `SignalHandle::reload_peer`
    // (M2-12, device deletion); the writer also calls `Persist::reload` on UuidMismatch.
    pub(crate) async fn reload(&self, id: &str) -> Result<(), StoreError> {
        self.persist.reload(id).await
    }
}
