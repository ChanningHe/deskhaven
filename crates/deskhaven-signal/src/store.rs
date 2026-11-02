//! Peer registry persistence (deskhaven code, not vendored): the [`PeerStore`] trait that
//! replaces upstream's `database.rs`, and the non-persistent [`MemoryPeerStore`].
//!
//! The server loads every registration at start ([`PeerStore::load_page`], page by page) and
//! from then on answers from memory only: the UDP and TCP paths never wait for the store.
//! Accepted registrations are written behind by one writer task (`persist.rs`), coalesced per
//! peer. The store is what makes the anti id-hijack check (UUID_MISMATCH) survive a restart:
//! an id registered once stays bound to its uuid. Rows of devices that never come online again
//! are kept; their retention comes with admission (M5) and the device lifecycle (M2-20).

use async_trait::async_trait;
use std::{
    collections::{BTreeMap, btree_map::Entry},
    net::IpAddr,
    ops::Bound,
    sync::{Mutex, MutexGuard, PoisonError},
    time::SystemTime,
};

type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// What the store knows about a registered peer (upstream `peer` row, reduced to what the
/// RegisterPk checks read).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerRecord {
    /// RustDesk id.
    pub id: String,
    /// Device uuid of the first accepted RegisterPk; never changes afterwards.
    pub uuid: Vec<u8>,
    /// Device public key of the last accepted RegisterPk.
    pub pk: Vec<u8>,
    /// Source IP of the last accepted RegisterPk (upstream `info.ip`); `None` if unknown.
    pub ip: Option<IpAddr>,
}

/// An accepted RegisterPk to persist.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerUpsert {
    /// RustDesk id (at most 253 bytes, checked by the server).
    pub id: String,
    pub uuid: Vec<u8>,
    pub pk: Vec<u8>,
    /// Source IP of the RegisterPk.
    pub ip: IpAddr,
    /// When the server accepted it.
    pub registered_at: SystemTime,
}

/// Why a store call failed.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// [`PeerStore::upsert_pk`] for an id that is stored with another uuid; nothing changed.
    #[error("peer {id} is registered with another uuid")]
    UuidMismatch { id: String },
    /// The backing storage failed (possibly transient; writes are retried).
    #[error("peer store failed")]
    Backend(#[source] BoxError),
}

impl StoreError {
    pub fn backend(err: impl Into<BoxError>) -> Self {
        Self::Backend(err.into())
    }
}

/// Persistent peer registry injected into [`SignalServer::new`](crate::SignalServer::new).
///
/// The server assumes it is the only writer: it loads everything once at start and keeps
/// memory authoritative afterwards. Registrations changed or deleted behind its back are not
/// seen until a restart (an explicit invalidation hook exists for later admin use).
#[async_trait]
pub trait PeerStore: Send + Sync + 'static {
    /// Up to `limit` stored registrations whose ids sort after `after` (from the first if
    /// `None`), in ascending id order as the store compares ids (keyset pagination: the server
    /// passes the last id of a page as the next `after`). Fewer than `limit` ends the load.
    /// Called page by page when the server starts, all pages together bounded by
    /// [`SignalConfig::peer_preload_timeout`](crate::SignalConfig::peer_preload_timeout).
    async fn load_page(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<PeerRecord>, StoreError>;

    /// The stored registration of `id`, if any. Not used for lookups while serving; only by
    /// the invalidation hook and after a [`StoreError::UuidMismatch`].
    async fn get(&self, id: &str) -> Result<Option<PeerRecord>, StoreError>;

    /// Inserts `rec`, or updates the stored pk / ip / registration time of `rec.id`. Called
    /// by one writer task, one call at a time, with the peer's state at the time of the call
    /// (registrations accepted in between are coalesced into one write).
    ///
    /// The uuid is immutable: if `rec.id` is stored with a different uuid, fail with
    /// [`StoreError::UuidMismatch`] and change nothing.
    async fn upsert_pk(&self, rec: &PeerUpsert) -> Result<(), StoreError>;
}

/// [`PeerStore`] in process memory. **Not persistent**: everything is lost when it is
/// dropped, so after a process restart any uuid can claim any id again. Only for tests and as
/// an explicit opt-in where that is acceptable; share one instance (`Arc`) across server
/// restarts within a process to keep registrations.
#[derive(Debug, Default)]
pub struct MemoryPeerStore {
    peers: Mutex<BTreeMap<String, PeerRecord>>,
}

impl MemoryPeerStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn peers(&self) -> MutexGuard<'_, BTreeMap<String, PeerRecord>> {
        // The map stays consistent even if a holder panicked: every update is one insert.
        self.peers.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[async_trait]
impl PeerStore for MemoryPeerStore {
    async fn load_page(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<PeerRecord>, StoreError> {
        let from = after.map_or(Bound::Unbounded, Bound::Excluded);
        let peers = self.peers();
        let page = peers.range::<str, _>((from, Bound::Unbounded));
        Ok(page.take(limit).map(|(_, rec)| rec.clone()).collect())
    }

    async fn get(&self, id: &str) -> Result<Option<PeerRecord>, StoreError> {
        Ok(self.peers().get(id).cloned())
    }

    async fn upsert_pk(&self, rec: &PeerUpsert) -> Result<(), StoreError> {
        let record = PeerRecord {
            id: rec.id.clone(),
            uuid: rec.uuid.clone(),
            pk: rec.pk.clone(),
            ip: Some(rec.ip),
        };
        match self.peers().entry(rec.id.clone()) {
            Entry::Vacant(slot) => {
                slot.insert(record);
            }
            Entry::Occupied(slot) if slot.get().uuid != rec.uuid => {
                return Err(StoreError::UuidMismatch { id: rec.id.clone() });
            }
            Entry::Occupied(mut slot) => {
                slot.insert(record);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hbb_common::tokio;
    use std::net::Ipv4Addr;

    fn upsert(id: &str, uuid: &[u8], pk: &[u8], ip: [u8; 4]) -> PeerUpsert {
        PeerUpsert {
            id: id.to_owned(),
            uuid: uuid.to_vec(),
            pk: pk.to_vec(),
            ip: IpAddr::V4(Ipv4Addr::from(ip)),
            registered_at: SystemTime::now(),
        }
    }

    #[tokio::test]
    async fn upsert_inserts_then_updates_pk_and_ip() {
        let store = MemoryPeerStore::new();
        assert_eq!(store.get("a").await.ok(), Some(None));

        store
            .upsert_pk(&upsert("a", b"u", b"pk1", [10, 0, 0, 1]))
            .await
            .ok();
        store
            .upsert_pk(&upsert("a", b"u", b"pk2", [10, 0, 0, 2]))
            .await
            .ok();

        let expected = PeerRecord {
            id: "a".to_owned(),
            uuid: b"u".to_vec(),
            pk: b"pk2".to_vec(),
            ip: Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2))),
        };
        assert_eq!(store.get("a").await.ok().flatten(), Some(expected.clone()));
        assert_eq!(store.load_page(None, 10).await.ok(), Some(vec![expected]));
    }

    #[tokio::test]
    async fn load_page_walks_ids_in_order_after_the_cursor() {
        let store = MemoryPeerStore::new();
        for id in ["c", "a", "e", "b", "d"] {
            store
                .upsert_pk(&upsert(id, b"u", b"pk", [10, 0, 0, 1]))
                .await
                .ok();
        }
        let ids = async |after: Option<&str>| -> Vec<String> {
            let page = store.load_page(after, 2).await.unwrap_or_default();
            page.into_iter().map(|r| r.id).collect()
        };

        assert_eq!(ids(None).await, ["a", "b"]);
        assert_eq!(ids(Some("b")).await, ["c", "d"]);
        assert_eq!(ids(Some("d")).await, ["e"]);
        assert!(ids(Some("e")).await.is_empty());
    }

    #[tokio::test]
    async fn upsert_with_other_uuid_fails_and_changes_nothing() {
        let store = MemoryPeerStore::new();
        store
            .upsert_pk(&upsert("a", b"u", b"pk1", [10, 0, 0, 1]))
            .await
            .ok();

        let res = store
            .upsert_pk(&upsert("a", b"x", b"pk2", [10, 0, 0, 2]))
            .await;

        assert!(matches!(res, Err(StoreError::UuidMismatch { id }) if id == "a"));
        let rec = store.get("a").await.ok().flatten();
        assert_eq!(
            rec.map(|r| (r.uuid, r.pk)),
            Some((b"u".to_vec(), b"pk1".to_vec()))
        );
    }
}
