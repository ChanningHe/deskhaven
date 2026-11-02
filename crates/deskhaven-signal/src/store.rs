//! Peer registry persistence (deskhaven code, not vendored): the [`PeerStore`] trait that
//! replaces upstream's `database.rs`, and the non-persistent [`MemoryPeerStore`].
//!
//! The server keeps every peer it has seen in memory and asks the store only for ids it does
//! not know yet (first RegisterPk / punch hole after a start); writes leave the UDP loop
//! through a bounded queue (see `peer.rs`). The store is what makes the anti id-hijack check
//! (UUID_MISMATCH) survive a restart: an id registered once stays bound to its uuid.

use async_trait::async_trait;
use std::{
    collections::{HashMap, hash_map::Entry},
    net::IpAddr,
    sync::{Mutex, PoisonError},
    time::SystemTime,
};

type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// What the store knows about a registered peer (upstream `peer` row, reduced to what the
/// RegisterPk checks read).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerRecord {
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
    /// The backing storage failed.
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
/// Neither method is called on the UDP loop for a peer already in memory: `get` runs once per
/// id and server start (cache miss), `upsert_pk` on a writer task, one call at a time, in the
/// order the server accepted the registrations.
#[async_trait]
pub trait PeerStore: Send + Sync + 'static {
    /// The stored registration of `id`, if any.
    async fn get(&self, id: &str) -> Result<Option<PeerRecord>, StoreError>;

    /// Inserts `rec`, or updates the stored pk / ip / registration time of `rec.id`.
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
    peers: Mutex<HashMap<String, PeerRecord>>,
}

impl MemoryPeerStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn peers(&self) -> std::sync::MutexGuard<'_, HashMap<String, PeerRecord>> {
        // The map stays consistent even if a holder panicked: every update is one insert.
        self.peers.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[async_trait]
impl PeerStore for MemoryPeerStore {
    async fn get(&self, id: &str) -> Result<Option<PeerRecord>, StoreError> {
        Ok(self.peers().get(id).cloned())
    }

    async fn upsert_pk(&self, rec: &PeerUpsert) -> Result<(), StoreError> {
        let record = PeerRecord {
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

        let rec = store.get("a").await.ok().flatten();
        assert_eq!(
            rec,
            Some(PeerRecord {
                uuid: b"u".to_vec(),
                pk: b"pk2".to_vec(),
                ip: Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2))),
            })
        );
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
