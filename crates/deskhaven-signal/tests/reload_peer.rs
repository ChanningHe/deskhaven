//! M2-12 review (M5): `SignalHandle::reload_peer` re-reads one registration from the store,
//! so a device deleted by the control plane is forgotten at once instead of answering
//! RegisterPk as registered (and the device getting ID_NOT_FOUND from the API) until restart.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use async_trait::async_trait;
use deskhaven_signal::{MemoryPeerStore, PeerRecord, PeerStore, PeerUpsert, StoreError};
use hbb_common::{
    rendezvous_proto::register_pk_response::Result::{OK, UUID_MISMATCH},
    tokio::{
        self,
        time::{Duration, Instant, sleep},
    },
};
use std::{
    collections::BTreeSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use support::{PeerKeys, STEP, TestServer};

const ID: &str = "reload-peer";
const POLL: Duration = Duration::from_millis(20);

/// [`MemoryPeerStore`] whose rows can be deleted behind the server's back.
#[derive(Default)]
struct DeletableStore {
    inner: MemoryPeerStore,
    deleted: Mutex<BTreeSet<String>>,
    /// Writes fail while set (the write-behind keeps them owed).
    failing: AtomicBool,
}

impl DeletableStore {
    fn delete(&self, id: &str) {
        self.deleted.lock().unwrap().insert(id.to_owned());
    }

    fn is_deleted(&self, id: &str) -> bool {
        self.deleted.lock().unwrap().contains(id)
    }
}

#[async_trait]
impl PeerStore for DeletableStore {
    async fn load_page(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<PeerRecord>, StoreError> {
        let page = self.inner.load_page(after, limit).await?;
        Ok(page
            .into_iter()
            .filter(|r| !self.is_deleted(&r.id))
            .collect())
    }

    async fn get(&self, id: &str) -> Result<Option<PeerRecord>, StoreError> {
        match self.is_deleted(id) {
            true => Ok(None),
            false => self.inner.get(id).await,
        }
    }

    async fn upsert_pk(&self, rec: &PeerUpsert) -> Result<(), StoreError> {
        if self.failing.load(Ordering::SeqCst) {
            return Err(StoreError::backend("store unavailable (test)"));
        }
        self.inner.upsert_pk(rec).await
    }
}

/// After the row is deleted and the peer reloaded, the id is free: a RegisterPeer is asked
/// for its key and another uuid can register it (a deleted device re-registers as new).
#[tokio::test]
async fn a_reloaded_deleted_peer_is_forgotten_and_registers_again() {
    // Arrange
    let store = Arc::new(DeletableStore::default());
    let server = TestServer::start_with_store(|cfg| cfg, store.clone()).await;
    let mut peer = server.udp().await;
    assert_eq!(peer.register_pk(ID, &PeerKeys::random()).await, OK);
    let mut other = server.udp().await;
    assert_eq!(
        other.register_pk(ID, &PeerKeys::random()).await,
        UUID_MISMATCH,
        "bound before the deletion"
    );

    // Act
    store.delete(ID);
    server.handle.reload_peer(ID).unwrap();

    // Assert
    let deadline = Instant::now() + STEP;
    while !peer.register_peer(ID).await.request_pk {
        assert!(Instant::now() < deadline, "the server still holds {ID}");
        sleep(POLL).await;
    }
    let mut fresh = server.udp().await;
    assert_eq!(fresh.register_pk(ID, &PeerKeys::random()).await, OK);
    server.stop().await;
}

/// Reloading an id the store still holds keeps it registered.
#[tokio::test]
async fn reloading_a_stored_peer_keeps_it() {
    // Arrange
    let store = Arc::new(DeletableStore::default());
    let server = TestServer::start_with_store(|cfg| cfg, store.clone()).await;
    let mut peer = server.udp().await;
    assert_eq!(peer.register_pk(ID, &PeerKeys::random()).await, OK);
    let deadline = Instant::now() + STEP;
    while store.inner.get(ID).await.unwrap().is_none() {
        assert!(Instant::now() < deadline, "registration never stored");
        sleep(POLL).await;
    }

    // Act
    server.handle.reload_peer(ID).unwrap();
    sleep(Duration::from_millis(200)).await;

    // Assert
    assert!(!peer.register_peer(ID).await.request_pk);
    let mut other = server.udp().await;
    assert_eq!(
        other.register_pk(ID, &PeerKeys::random()).await,
        UUID_MISMATCH
    );
    server.stop().await;
}

/// M2-19 re-review: a reinstalled device re-registers with a new key, then a reload arrives
/// (an out-of-band registry write) before the write-behind stored that key. The newer
/// in-memory key of the same device wins: it is kept and still written, not reverted to the
/// stored one.
#[tokio::test]
async fn reloading_keeps_a_newer_unsaved_key_of_the_same_device() {
    // Arrange
    let store = Arc::new(DeletableStore::default());
    let server = TestServer::start_with_store(|cfg| cfg, store.clone()).await;
    let first = PeerKeys::random();
    let mut peer = server.udp().await;
    assert_eq!(peer.register_pk(ID, &first).await, OK);
    let deadline = Instant::now() + STEP;
    while store.inner.get(ID).await.unwrap().is_none() {
        assert!(Instant::now() < deadline, "registration never stored");
        sleep(POLL).await;
    }
    store.failing.store(true, Ordering::SeqCst);
    let reinstalled = first.with_new_pk();
    assert_eq!(peer.register_pk(ID, &reinstalled).await, OK);

    // Act
    server.handle.reload_peer(ID).unwrap();
    sleep(Duration::from_millis(200)).await;
    store.failing.store(false, Ordering::SeqCst);

    // Assert
    let deadline = Instant::now() + 3 * STEP;
    loop {
        let stored = store.inner.get(ID).await.unwrap().unwrap();
        if stored.pk == reinstalled.pk.to_vec() {
            break;
        }
        assert!(Instant::now() < deadline, "the new key was never written");
        sleep(POLL).await;
    }
    server.stop().await;
}
