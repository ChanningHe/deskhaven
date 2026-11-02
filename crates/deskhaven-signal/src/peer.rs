// Modified by deskhaven; see crates/deskhaven-signal/CHANGES.md (AGPL-3.0 §5a).
use crate::common::*;
use crate::persist::{PeerWrite, Persist};
use crate::store::{PeerStore, PeerUpsert, StoreError};
use crate::tasks::Tasks;
use hbb_common::{
    bytes::Bytes,
    log,
    tokio::{
        sync::{Mutex, RwLock, mpsc::error::TrySendError},
        time::Duration,
    },
};
use std::{
    collections::HashMap,
    collections::HashSet,
    net::SocketAddr,
    sync::Arc,
    time::{Instant, SystemTime},
};

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
    // deskhaven: the accepted registration could not be saved (persist.rs); ask the client for
    // it again so the write is retried.
    pub(crate) dirty: bool,
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
            dirty: false,
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
        let map = PeerCache::default();
        let persist = Persist::start(store, map.clone(), tasks);
        Self { map, persist }
    }

    // deskhaven: loads every stored registration (startup, before serving).
    pub(crate) async fn preload(&self, limit: Duration) -> Result<usize, StoreError> {
        self.persist.preload(limit).await
    }

    // deskhaven: the store write is queued, not awaited. Fails, before touching the peer,
    // only when the queue is full or the writer has stopped; the caller then refuses the
    // registration. Upstream awaited the database and ignored the result.
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
        let permit = self.persist.reserve()?;
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

    // deskhaven: invalidation hook for out-of-band registry changes (admin, M2); not wired.
    #[allow(dead_code)]
    pub(crate) async fn reload(&self, id: &str) -> Result<(), StoreError> {
        self.persist.reload(id).await
    }
}
