//! Preload, write-behind ordering, retries, eviction and the stop deadline. Time-dependent
//! tests run on tokio's paused clock (sleeps and timeouts advance instantly when idle).

use super::*;
use crate::peer::PeerMap;
use crate::store::MemoryPeerStore;
use async_trait::async_trait;
use hbb_common::bytes::Bytes;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::SystemTime,
};
use tokio::time::Instant;

const ID: &str = "123456789";
const UUID: &[u8] = b"owner-uuid";
const PK1: &[u8] = &[1; 32];
const PK2: &[u8] = &[2; 32];
/// Upper bound for anything the writer must get done in a test (paused clock).
const SETTLE: Duration = Duration::from_secs(60);

/// A memory store with scripted failures, recording the pk of every upsert attempt.
#[derive(Default)]
struct ScriptedStore {
    inner: MemoryPeerStore,
    /// Upcoming upserts that fail with a backend error.
    fail_next: AtomicUsize,
    /// Upserts of this pk always fail with a backend error.
    fail_pk: Mutex<Option<Vec<u8>>>,
    /// Upserts and loads never complete.
    hang: AtomicBool,
    attempts: Mutex<Vec<Vec<u8>>>,
    gets: AtomicUsize,
    /// Ids `get` no longer finds (deleted out of band).
    deleted: Mutex<Vec<String>>,
}

impl ScriptedStore {
    fn attempts(&self) -> Vec<Vec<u8>> {
        self.attempts.lock().unwrap().clone()
    }

    async fn hang_if_asked(&self) {
        if self.hang.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
    }
}

#[async_trait]
impl PeerStore for ScriptedStore {
    async fn load_all(&self) -> Result<Vec<PeerRecord>, StoreError> {
        self.hang_if_asked().await;
        self.inner.load_all().await
    }

    async fn get(&self, id: &str) -> Result<Option<PeerRecord>, StoreError> {
        self.gets.fetch_add(1, Ordering::SeqCst);
        if self.deleted.lock().unwrap().iter().any(|d| d == id) {
            return Ok(None);
        }
        self.inner.get(id).await
    }

    async fn upsert_pk(&self, rec: &PeerUpsert) -> Result<(), StoreError> {
        self.attempts.lock().unwrap().push(rec.pk.clone());
        self.hang_if_asked().await;
        let scripted = self.fail_next.load(Ordering::SeqCst) > 0;
        if scripted {
            self.fail_next.fetch_sub(1, Ordering::SeqCst);
        }
        let pk_fails = self.fail_pk.lock().unwrap().as_deref() == Some(rec.pk.as_slice());
        if scripted || pk_fails {
            return Err(StoreError::backend("injected failure"));
        }
        self.inner.upsert_pk(rec).await
    }
}

fn addr() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 5)), 40000)
}

fn record(id: &str, uuid: &[u8], pk: &[u8]) -> PeerUpsert {
    PeerUpsert {
        id: id.to_owned(),
        uuid: uuid.to_vec(),
        pk: pk.to_vec(),
        ip: addr().ip(),
        registered_at: SystemTime::now(),
    }
}

fn server(store: &Arc<ScriptedStore>) -> (PeerMap, Tasks) {
    let tasks = Tasks::new(CancellationToken::new());
    let pm = PeerMap::new(store.clone(), &tasks);
    (pm, tasks)
}

/// What the RegisterPk path does for an accepted registration.
async fn accept(
    pm: &mut PeerMap,
    id: &str,
    uuid: &[u8],
    pk: &[u8],
) -> Result<(), TrySendError<()>> {
    let peer = pm.get_or(id).await;
    let ip = addr().ip().to_string();
    let (uuid, pk) = (Bytes::copy_from_slice(uuid), Bytes::copy_from_slice(pk));
    pm.update_pk(id.to_owned(), peer, addr(), uuid, pk, ip)
        .await
}

/// Polls `done` on the paused clock until it holds; panics after [`SETTLE`].
async fn eventually<F: AsyncFn() -> bool>(what: &str, done: F) {
    let deadline = Instant::now() + SETTLE;
    while !done().await {
        assert!(Instant::now() < deadline, "never happened: {what}");
        sleep(Duration::from_millis(10)).await;
    }
}

async fn stored_pk(store: &ScriptedStore, id: &str) -> Option<Vec<u8>> {
    store.inner.get(id).await.unwrap().map(|r| r.pk)
}

async fn cached_pk(pm: &PeerMap, id: &str) -> Option<Bytes> {
    Some(pm.get(id).await?.read().await.pk.clone())
}

#[tokio::test]
async fn stop_flushes_queued_writes_to_the_store() {
    let store = Arc::new(ScriptedStore::default());
    let (mut pm, tasks) = server(&store);

    accept(&mut pm, ID, UUID, PK1).await.unwrap();
    tasks.shutdown().await;

    assert_eq!(stored_pk(&store, ID).await.as_deref(), Some(PK1));
}

#[tokio::test]
async fn preload_puts_every_stored_registration_in_memory() {
    let store = Arc::new(ScriptedStore::default());
    for n in 0..100 {
        store
            .inner
            .upsert_pk(&record(&format!("peer-{n}"), UUID, PK1))
            .await
            .unwrap();
    }
    let (pm, tasks) = server(&store);

    let loaded = pm.preload(SETTLE).await.unwrap();

    assert_eq!(loaded, 100);
    let peer = pm.get("peer-42").await.unwrap();
    let peer = peer.read().await;
    assert_eq!((peer.uuid.as_ref(), peer.pk.as_ref()), (UUID, PK1));
    assert_eq!(peer.info.ip, addr().ip().to_string());
    assert!(pm.get("peer-100").await.is_none());
    tasks.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn preload_fails_when_the_store_is_too_slow() {
    let store = Arc::new(ScriptedStore::default());
    store.hang.store(true, Ordering::SeqCst);
    let (pm, tasks) = server(&store);

    let res = pm.preload(Duration::from_secs(1)).await;

    assert!(matches!(res, Err(StoreError::Backend(_))), "{res:?}");
    tasks.shutdown().await;
}

/// Lookups after the preload are memory only: hits and misses alike.
#[tokio::test]
async fn lookups_never_ask_the_store() {
    let store = Arc::new(ScriptedStore::default());
    store.inner.upsert_pk(&record(ID, UUID, PK1)).await.unwrap();
    let (pm, tasks) = server(&store);
    pm.preload(SETTLE).await.unwrap();

    assert!(pm.get(ID).await.is_some());
    assert!(pm.get("unknown-peer").await.is_none());
    pm.get_or("new-peer").await;

    assert_eq!(store.gets.load(Ordering::SeqCst), 0);
    tasks.shutdown().await;
}

/// Scenario a: a transient failure keeps the accepted (owner's) entry in memory, so another
/// uuid is still checked against it; the write is retried until it lands.
#[tokio::test(start_paused = true)]
async fn transient_backend_failure_is_retried_and_keeps_the_peer() {
    let store = Arc::new(ScriptedStore::default());
    store.fail_next.store(2, Ordering::SeqCst);
    let (mut pm, tasks) = server(&store);

    accept(&mut pm, ID, UUID, PK1).await.unwrap();
    eventually("write lands", async || {
        stored_pk(&store, ID).await.is_some()
    })
    .await;

    assert_eq!(store.attempts().len(), 3);
    let peer = pm.get(ID).await.expect("kept in memory");
    let peer = peer.read().await;
    assert_eq!((peer.uuid.as_ref(), peer.pk.as_ref()), (UUID, PK1));
    assert!(!peer.dirty);
    drop(peer);
    tasks.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn exhausted_retries_keep_the_peer_and_mark_it_dirty() {
    let store = Arc::new(ScriptedStore::default());
    store.fail_next.store(usize::MAX, Ordering::SeqCst);
    let (mut pm, tasks) = server(&store);

    accept(&mut pm, ID, UUID, PK1).await.unwrap();
    let peer = pm.get(ID).await.unwrap();
    eventually("marked dirty", async || peer.read().await.dirty).await;

    assert_eq!(store.attempts().len(), RETRY_BACKOFF.len() + 1);
    assert!(
        pm.get(ID).await.is_some_and(|p| Arc::ptr_eq(&p, &peer)),
        "not evicted"
    );
    assert_eq!(stored_pk(&store, ID).await, None);
    tasks.shutdown().await;
}

/// Scenario b: W1 (pk1) fails while W2 (pk2, same device) is queued. W1 is not retried after
/// W2 and cannot overwrite it; store and memory end with pk2, the only pk ever served besides
/// pk1 (the cached keys never leave the accepted registrations).
#[tokio::test(start_paused = true)]
async fn failed_write_superseded_by_a_queued_newer_write_is_not_retried() {
    let store = Arc::new(ScriptedStore::default());
    *store.fail_pk.lock().unwrap() = Some(PK1.to_vec());
    let (mut pm, tasks) = server(&store);

    accept(&mut pm, ID, UUID, PK1).await.unwrap();
    accept(&mut pm, ID, UUID, PK2).await.unwrap();
    eventually("W2 lands", async || stored_pk(&store, ID).await.is_some()).await;
    sleep(SETTLE).await;

    assert_eq!(store.attempts(), vec![PK1.to_vec(), PK2.to_vec()]);
    assert_eq!(stored_pk(&store, ID).await.as_deref(), Some(PK2));
    assert_eq!(cached_pk(&pm, ID).await.as_deref(), Some(PK2));
    assert!(!pm.get(ID).await.unwrap().read().await.dirty);
    tasks.shutdown().await;
}

/// The store binds the id to another uuid (changed out of band after the preload): memory
/// yields, so the next RegisterPk is checked against the store again after a reload/restart
/// instead of serving keys the store refused.
#[tokio::test]
async fn uuid_mismatch_from_the_store_evicts_the_peer() {
    let store = Arc::new(ScriptedStore::default());
    let (mut pm, tasks) = server(&store);
    pm.preload(SETTLE).await.unwrap();
    store
        .inner
        .upsert_pk(&record(ID, b"other-uuid", PK2))
        .await
        .unwrap();

    accept(&mut pm, ID, UUID, PK1).await.unwrap();
    tasks.shutdown().await;

    assert!(pm.get(ID).await.is_none(), "evicted");
    assert_eq!(stored_pk(&store, ID).await.as_deref(), Some(PK2));
}

/// One deadline covers the whole flush: a write stuck in the store is cancelled with it.
#[tokio::test(start_paused = true)]
async fn stop_deadline_cancels_an_in_flight_write() {
    let store = Arc::new(ScriptedStore::default());
    store.hang.store(true, Ordering::SeqCst);
    let (mut pm, tasks) = server(&store);
    accept(&mut pm, ID, UUID, PK1).await.unwrap();
    eventually("write in flight", async || !store.attempts().is_empty()).await;

    let start = Instant::now();
    tasks.shutdown().await;

    let took = start.elapsed();
    assert!(
        took >= DRAIN_TIMEOUT && took < DRAIN_TIMEOUT + Duration::from_secs(1),
        "{took:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn full_queue_refuses_before_touching_the_peer() {
    let store = Arc::new(ScriptedStore::default());
    store.hang.store(true, Ordering::SeqCst);
    let (mut pm, tasks) = server(&store);

    // One write in flight (stuck), then the queue fills up.
    let mut refused = None;
    for n in 0..=WRITE_QUEUE + 1 {
        let id = format!("peer-{n}");
        if accept(&mut pm, &id, UUID, PK1).await.is_err() {
            refused = Some(id);
            break;
        }
    }

    let id = refused.expect("a registration was refused");
    let peer = pm.get(&id).await.expect("entry created by get_or");
    assert!(peer.read().await.uuid.is_empty(), "untouched");
    drop(peer);
    tasks.shutdown().await;
}

#[tokio::test]
async fn reload_replaces_cached_keys_or_forgets_the_id() {
    let store = Arc::new(ScriptedStore::default());
    store.inner.upsert_pk(&record(ID, UUID, PK1)).await.unwrap();
    store
        .inner
        .upsert_pk(&record("gone-peer", UUID, PK1))
        .await
        .unwrap();
    let (pm, tasks) = server(&store);
    pm.preload(SETTLE).await.unwrap();
    // Changed and deleted out of band.
    store.inner.upsert_pk(&record(ID, UUID, PK2)).await.unwrap();
    store.deleted.lock().unwrap().push("gone-peer".to_owned());

    pm.reload(ID).await.unwrap();
    pm.reload("gone-peer").await.unwrap();

    assert_eq!(cached_pk(&pm, ID).await.as_deref(), Some(PK2));
    assert!(pm.get("gone-peer").await.is_none());
    tasks.shutdown().await;
}
