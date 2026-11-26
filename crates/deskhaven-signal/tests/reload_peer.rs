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
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use support::{PeerKeys, STEP, TestServer};

const ID: &str = "reload-peer";
const POLL: Duration = Duration::from_millis(20);

#[derive(Default)]
struct DeletableStore {
    inner: MemoryPeerStore,
    deleted: Mutex<BTreeSet<String>>,
    /// Writes fail while set (the write-behind keeps them owed).
    failing: AtomicBool,
    /// Completed `get`s: a reload has read the store once this grows.
    gets: AtomicUsize,
}

impl DeletableStore {
    fn delete(&self, id: &str) {
        self.deleted.lock().unwrap().insert(id.to_owned());
    }

    fn is_deleted(&self, id: &str) -> bool {
        self.deleted.lock().unwrap().contains(id)
    }

    async fn reload(&self, server: &TestServer) {
        let before = self.gets.load(Ordering::SeqCst);
        server.handle.reload_peer(ID).unwrap();
        let deadline = Instant::now() + STEP;
        while self.gets.load(Ordering::SeqCst) == before {
            assert!(Instant::now() < deadline, "the reload never read the store");
            sleep(POLL).await;
        }
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
        let found = match self.is_deleted(id) {
            true => Ok(None),
            false => self.inner.get(id).await,
        };
        self.gets.fetch_add(1, Ordering::SeqCst);
        found
    }

    async fn upsert_pk(&self, rec: &PeerUpsert) -> Result<(), StoreError> {
        if self.failing.load(Ordering::SeqCst) {
            return Err(StoreError::backend("store unavailable (test)"));
        }
        self.inner.upsert_pk(rec).await
    }
}

#[tokio::test]
async fn a_reloaded_deleted_peer_is_forgotten_and_registers_again() {
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

    store.delete(ID);
    server.handle.reload_peer(ID).unwrap();

    let deadline = Instant::now() + STEP;
    while !peer.register_peer(ID).await.request_pk {
        assert!(Instant::now() < deadline, "the server still holds {ID}");
        sleep(POLL).await;
    }
    let mut fresh = server.udp().await;
    assert_eq!(fresh.register_pk(ID, &PeerKeys::random()).await, OK);
    server.stop().await;
}

#[tokio::test]
async fn reloading_a_stored_peer_keeps_it() {
    let store = Arc::new(DeletableStore::default());
    let server = TestServer::start_with_store(|cfg| cfg, store.clone()).await;
    let mut peer = server.udp().await;
    assert_eq!(peer.register_pk(ID, &PeerKeys::random()).await, OK);
    let deadline = Instant::now() + STEP;
    while store.inner.get(ID).await.unwrap().is_none() {
        assert!(Instant::now() < deadline, "registration never stored");
        sleep(POLL).await;
    }

    store.reload(&server).await;

    assert!(!peer.register_peer(ID).await.request_pk);
    let mut other = server.udp().await;
    assert_eq!(
        other.register_pk(ID, &PeerKeys::random()).await,
        UUID_MISMATCH
    );
    server.stop().await;
}

#[tokio::test]
async fn reloading_keeps_a_newer_unsaved_key_of_the_same_device() {
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

    store.reload(&server).await;
    store.failing.store(false, Ordering::SeqCst);

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
