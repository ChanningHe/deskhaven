//! Preload, coalesced write-behind, store outages (circuit breaker), UuidMismatch rebinding
//! and the stop. Time-dependent tests run on tokio's paused clock (sleeps and timeouts
//! advance instantly when every task is idle).

use super::*;
use crate::peer::PeerMap;
use crate::store::MemoryPeerStore;
use async_trait::async_trait;
use hbb_common::bytes::Bytes;
use hbb_common::tokio::time::sleep_until;
use std::net::{Ipv4Addr, SocketAddr};

const ID: &str = "123456789";
const UUID: &[u8] = b"owner-uuid";
const PK1: &[u8] = &[1; 32];
const PK2: &[u8] = &[2; 32];
/// Upper bound for anything the writer must get done in a test (paused clock).
const SETTLE: Duration = Duration::from_secs(120);

/// A memory store with scripted failures, recording every upsert attempt.
#[derive(Default)]
struct ScriptedStore {
    inner: MemoryPeerStore,
    /// Upcoming upserts that fail with a backend error (`usize::MAX`: store down).
    fail_next: AtomicUsize,
    /// Upserts and loads never complete.
    hang: AtomicBool,
    /// (id, pk) of every upsert attempt.
    attempts: Mutex<Vec<(String, Vec<u8>)>>,
    gets: AtomicUsize,
    /// (after, limit) of every `load_page` call.
    pages: Mutex<Vec<(Option<String>, usize)>>,
    /// Ids `get` no longer finds (deleted out of band).
    deleted: Mutex<Vec<String>>,
}

impl ScriptedStore {
    fn attempts(&self) -> Vec<(String, Vec<u8>)> {
        self.attempts.lock().unwrap().clone()
    }

    fn attempt_count(&self) -> usize {
        self.attempts.lock().unwrap().len()
    }

    fn set_down(&self, down: bool) {
        self.fail_next
            .store(if down { usize::MAX } else { 0 }, Ordering::SeqCst);
    }

    async fn hang_if_asked(&self) {
        if self.hang.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
    }
}

#[async_trait]
impl PeerStore for ScriptedStore {
    async fn load_page(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<PeerRecord>, StoreError> {
        self.pages
            .lock()
            .unwrap()
            .push((after.map(str::to_owned), limit));
        self.hang_if_asked().await;
        self.inner.load_page(after, limit).await
    }

    async fn get(&self, id: &str) -> Result<Option<PeerRecord>, StoreError> {
        self.gets.fetch_add(1, Ordering::SeqCst);
        if self.deleted.lock().unwrap().iter().any(|d| d == id) {
            return Ok(None);
        }
        self.inner.get(id).await
    }

    async fn upsert_pk(&self, rec: &PeerUpsert) -> Result<(), StoreError> {
        self.attempts
            .lock()
            .unwrap()
            .push((rec.id.clone(), rec.pk.clone()));
        self.hang_if_asked().await;
        // One writer task, one call at a time: load-then-sub cannot race.
        if self.fail_next.load(Ordering::SeqCst) > 0 {
            self.fail_next.fetch_sub(1, Ordering::SeqCst);
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
    server_with_queue(store, WRITE_QUEUE)
}

fn server_with_queue(store: &Arc<ScriptedStore>, cap: usize) -> (PeerMap, Tasks) {
    let tasks = Tasks::new(CancellationToken::new());
    let pm = PeerMap::with_write_queue(store.clone(), &tasks, cap);
    (pm, tasks)
}

/// What the RegisterPk path does for an accepted registration from `from`.
async fn accept_from(
    pm: &mut PeerMap,
    id: &str,
    uuid: &[u8],
    pk: &[u8],
    from: SocketAddr,
) -> Result<(), QueueFull> {
    let peer = pm.get_or(id).await;
    let (uuid, pk) = (Bytes::copy_from_slice(uuid), Bytes::copy_from_slice(pk));
    let ip = from.ip().to_string();
    pm.update_pk(id.to_owned(), peer, from, uuid, pk, ip).await
}

async fn accept(pm: &mut PeerMap, id: &str, uuid: &[u8], pk: &[u8]) -> Result<(), QueueFull> {
    accept_from(pm, id, uuid, pk, addr()).await
}

/// Polls `done` until it holds; panics after [`SETTLE`] (paused clock: instant).
async fn eventually<F: AsyncFn() -> bool>(what: &str, done: F) {
    let deadline = Instant::now() + SETTLE;
    while !done().await {
        assert!(Instant::now() < deadline, "never happened: {what}");
        sleep(Duration::from_millis(10)).await;
    }
}

async fn stored(store: &ScriptedStore, id: &str) -> Option<PeerRecord> {
    store.inner.get(id).await.unwrap()
}

async fn stored_pk(store: &ScriptedStore, id: &str) -> Option<Vec<u8>> {
    stored(store, id).await.map(|r| r.pk)
}

async fn cached_pk(pm: &PeerMap, id: &str) -> Option<Bytes> {
    Some(pm.get(id).await?.read().await.pk.clone())
}

fn peer_ids(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("peer-{i:04}")).collect()
}

#[tokio::test]
async fn stop_flushes_queued_writes_to_the_store() {
    let store = Arc::new(ScriptedStore::default());
    let (mut pm, tasks) = server(&store);

    accept(&mut pm, ID, UUID, PK1).await.unwrap();
    tasks.shutdown().await;

    assert_eq!(stored_pk(&store, ID).await.as_deref(), Some(PK1));
    assert_eq!(pm.persist().lost_at_stop(), 0);
}

/// M-B: the preload walks the store in pages of [`PRELOAD_PAGE`], each starting after the last
/// id of the previous one, and ends on a short page.
#[tokio::test]
async fn preload_streams_pages_with_a_keyset_cursor() {
    let store = Arc::new(ScriptedStore::default());
    let ids = peer_ids(2 * PRELOAD_PAGE + 500);
    for id in &ids {
        store.inner.upsert_pk(&record(id, UUID, PK1)).await.unwrap();
    }
    let (pm, tasks) = server(&store);

    let loaded = pm.preload(SETTLE).await.unwrap();

    assert_eq!(loaded, ids.len());
    let calls = store.pages.lock().unwrap().clone();
    let expected = vec![
        (None, PRELOAD_PAGE),
        (Some(ids[PRELOAD_PAGE - 1].clone()), PRELOAD_PAGE),
        (Some(ids[2 * PRELOAD_PAGE - 1].clone()), PRELOAD_PAGE),
    ];
    assert_eq!(calls, expected);
    let peer = pm.get(&ids[1234]).await.unwrap();
    let peer = peer.read().await;
    assert_eq!((peer.uuid.as_ref(), peer.pk.as_ref()), (UUID, PK1));
    assert_eq!(peer.info.ip, addr().ip().to_string());
    assert!(peer.save.is_saved());
    drop(peer);
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

/// A transient failure keeps the accepted entry in memory; the writer retries it itself
/// until it lands.
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

    assert_eq!(store.attempt_count(), 3);
    let peer = pm.get(ID).await.expect("kept in memory");
    let peer = peer.read().await;
    assert_eq!((peer.uuid.as_ref(), peer.pk.as_ref()), (UUID, PK1));
    assert!(peer.save.is_saved());
    drop(peer);
    tasks.shutdown().await;
}

/// H-A.1: registrations accepted before the writer gets to the id collapse into one write
/// of the latest state.
#[tokio::test]
async fn registrations_before_the_write_collapse_into_the_latest_state() {
    let store = Arc::new(ScriptedStore::default());
    let (mut pm, tasks) = server(&store);

    accept(&mut pm, ID, UUID, PK1).await.unwrap();
    accept(&mut pm, ID, UUID, PK2).await.unwrap();
    assert_eq!(pm.persist().queued_len(), 1);
    tasks.shutdown().await;

    assert_eq!(store.attempts(), vec![(ID.to_owned(), PK2.to_vec())]);
    assert_eq!(stored_pk(&store, ID).await.as_deref(), Some(PK2));
}

/// H-A.1: while the store is down, any number of registrations of one peer keep exactly one
/// pending write; after the recovery that peer is written exactly once, with its latest pk.
#[tokio::test(start_paused = true)]
async fn duplicate_writes_are_impossible_during_an_outage() {
    let store = Arc::new(ScriptedStore::default());
    store.set_down(true);
    let (mut pm, tasks) = server(&store);
    accept(&mut pm, ID, UUID, PK1).await.unwrap();
    eventually("breaker open", async || {
        store.attempt_count() >= BREAKER_THRESHOLD as usize
    })
    .await;

    for n in 0..100u8 {
        accept(&mut pm, ID, UUID, &[n; 32]).await.unwrap();
        assert_eq!(pm.persist().queued_len(), 1, "registration {n}");
    }
    let before = store.attempt_count();
    store.set_down(false);
    eventually("saved", async || stored_pk(&store, ID).await.is_some()).await;
    sleep(SETTLE).await;

    let after: Vec<_> = store.attempts().split_off(before);
    assert_eq!(after, vec![(ID.to_owned(), vec![99; 32])]);
    assert!(pm.get(ID).await.unwrap().read().await.save.is_saved());
    tasks.shutdown().await;
}

/// H-A.4: after [`BREAKER_THRESHOLD`] consecutive failures no write is attempted for the
/// cool-down; a failed probe doubles it; a successful probe closes the breaker and the
/// backlog is written at once, each peer once.
#[tokio::test(start_paused = true)]
async fn breaker_opens_after_consecutive_failures_and_closes_after_a_successful_probe() {
    const PEERS: usize = 20;
    let store = Arc::new(ScriptedStore::default());
    store.set_down(true);
    let (mut pm, tasks) = server(&store);
    let start = Instant::now();
    for id in peer_ids(PEERS) {
        accept(&mut pm, &id, UUID, PK1).await.unwrap();
    }

    // Open: three attempts, then nothing for COOL_DOWN_MIN (1 s).
    sleep(COOL_DOWN_MIN / 2).await;
    assert_eq!(store.attempt_count(), BREAKER_THRESHOLD as usize);
    // The probe at 1 s fails: open again, for 2 s.
    sleep_until(start + COOL_DOWN_MIN + COOL_DOWN_MIN / 2).await;
    assert_eq!(store.attempt_count(), BREAKER_THRESHOLD as usize + 1);
    sleep_until(start + COOL_DOWN_MIN * 3 - Duration::from_millis(100)).await;
    assert_eq!(store.attempt_count(), BREAKER_THRESHOLD as usize + 1);

    // Recovered: the probe at 3 s succeeds and closes the breaker.
    store.set_down(false);
    sleep_until(start + COOL_DOWN_MIN * 3 + Duration::from_millis(100)).await;

    for id in peer_ids(PEERS) {
        assert!(stored(&store, &id).await.is_some(), "{id} saved");
    }
    assert_eq!(
        store.attempt_count(),
        BREAKER_THRESHOLD as usize + 1 + PEERS
    );
    assert_eq!(pm.persist().queued_len(), 0);
    tasks.shutdown().await;
}

/// H-A: a fleet keeps re-registering (new IPs) through a 10-minute outage. Every registration
/// is accepted and served from memory, the queue never holds more than one entry per peer,
/// the breaker keeps attempts to a few per minute, and after the recovery every peer is saved
/// with its latest address, written once.
#[tokio::test(start_paused = true)]
async fn fleet_outage_keeps_peers_accepted_and_the_queue_bounded() {
    const PEERS: usize = 50;
    const ROUNDS: u8 = 60;
    let ids = peer_ids(PEERS);
    let from = |round: u8, n: usize| {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, round, n as u8)), 40000)
    };
    let store = Arc::new(ScriptedStore::default());
    let (mut pm, tasks) = server(&store);
    for (n, id) in ids.iter().enumerate() {
        accept_from(&mut pm, id, UUID, PK1, from(0, n))
            .await
            .unwrap();
    }
    eventually("fleet saved", async || {
        pm.persist().queued_len() == 0 && store.attempt_count() == PEERS
    })
    .await;

    store.set_down(true);
    for round in 1..=ROUNDS {
        for (n, id) in ids.iter().enumerate() {
            let res = accept_from(&mut pm, id, UUID, PK1, from(round, n)).await;
            assert!(res.is_ok(), "round {round}: {id} refused");
            let peer = pm.get(id).await.unwrap();
            assert_eq!(
                peer.read().await.socket_addr,
                from(round, n),
                "served from memory"
            );
        }
        assert!(pm.persist().queued_len() <= PEERS, "round {round}");
        sleep(Duration::from_secs(10)).await;
    }
    let outage_attempts = store.attempt_count() - PEERS;
    assert!(
        outage_attempts <= 30,
        "breaker: {outage_attempts} attempts in 10 min"
    );

    store.set_down(false);
    eventually("backlog saved", async || pm.persist().queued_len() == 0).await;
    sleep(COOL_DOWN_MAX).await;

    let recovery_attempts = store.attempt_count() - PEERS - outage_attempts;
    assert_eq!(recovery_attempts, PEERS, "each peer written once");
    for (n, id) in ids.iter().enumerate() {
        let rec = stored(&store, id).await.unwrap();
        assert_eq!(rec.ip, Some(from(ROUNDS, n).ip()), "{id}");
    }
    tasks.shutdown().await;
}

/// H-A.3: a full queue still accepts bound peers (memory updated, write owed) and refuses
/// only ids that are not bound yet, before touching them.
#[tokio::test(start_paused = true)]
async fn full_queue_accepts_bound_peers_and_refuses_only_new_ids() {
    let store = Arc::new(ScriptedStore::default());
    store.hang.store(true, Ordering::SeqCst);
    let (mut pm, tasks) = server_with_queue(&store, 2);
    // "a" is in flight (stuck), "b" and "c" fill the queue.
    accept(&mut pm, "peer-a", UUID, PK1).await.unwrap();
    eventually("a in flight", async || store.attempt_count() == 1).await;
    accept(&mut pm, "peer-b", UUID, PK1).await.unwrap();
    accept(&mut pm, "peer-c", UUID, PK1).await.unwrap();

    let new_id = accept(&mut pm, "peer-d", UUID, PK1).await;
    let bound_queued = accept(&mut pm, "peer-b", UUID, PK2).await;
    let bound_in_flight = accept(&mut pm, "peer-a", UUID, PK2).await;

    assert!(new_id.is_err(), "new id refused");
    let d = pm.get("peer-d").await.expect("entry created by get_or");
    assert!(d.read().await.uuid.is_empty(), "untouched");
    assert!(
        bound_queued.is_ok() && bound_in_flight.is_ok(),
        "bound peers accepted"
    );
    for id in ["peer-a", "peer-b"] {
        assert_eq!(cached_pk(&pm, id).await.as_deref(), Some(PK2), "{id}");
        assert!(
            !pm.get(id).await.unwrap().read().await.save.is_saved(),
            "{id} owed"
        );
    }
    assert_eq!(pm.persist().queued_len(), 3, "b, c and a again");
    tasks.shutdown().await;
}

/// M-A: the store binds the id to another uuid (changed out of band after the preload). Its
/// binding is loaded into the cached peer in place: same entry, the store's keys, the
/// rate-limit counter kept, nothing owed.
#[tokio::test]
async fn uuid_mismatch_rebinds_the_store_record_in_place() {
    let store = Arc::new(ScriptedStore::default());
    let (mut pm, tasks) = server(&store);
    pm.preload(SETTLE).await.unwrap();
    store
        .inner
        .upsert_pk(&record(ID, b"other-uuid", PK2))
        .await
        .unwrap();
    let peer = pm.get_or(ID).await;
    let counter = (2, std::time::Instant::now());
    peer.write().await.reg_pk = counter;

    accept(&mut pm, ID, UUID, PK1).await.unwrap();
    eventually("rebound", async || {
        peer.read().await.uuid.as_ref() == b"other-uuid"
    })
    .await;

    assert!(Arc::ptr_eq(&pm.get(ID).await.unwrap(), &peer), "same entry");
    let p = peer.read().await;
    assert_eq!(p.pk.as_ref(), PK2);
    assert_eq!(p.reg_pk, counter);
    assert!(p.save.is_saved());
    drop(p);
    tasks.shutdown().await;
}

/// LOW: one deadline covers the whole flush; a write stuck in the store is cancelled with it
/// and, like everything still queued, counted as unsaved.
#[tokio::test(start_paused = true)]
async fn stop_deadline_cancels_an_in_flight_write_and_counts_the_unsaved() {
    let store = Arc::new(ScriptedStore::default());
    store.hang.store(true, Ordering::SeqCst);
    let (mut pm, tasks) = server(&store);
    for id in peer_ids(3) {
        accept(&mut pm, &id, UUID, PK1).await.unwrap();
    }
    eventually("write in flight", async || store.attempt_count() == 1).await;

    let start = Instant::now();
    tasks.shutdown().await;

    let took = start.elapsed();
    assert!(
        took >= DRAIN_TIMEOUT && took < DRAIN_TIMEOUT + Duration::from_secs(1),
        "{took:?}"
    );
    assert_eq!(pm.persist().lost_at_stop(), 3);
}

/// LOW: on stop each queued write gets one attempt, without breaker or backoff, even while the
/// breaker is open; the failed ones are counted as unsaved.
#[tokio::test(start_paused = true)]
async fn stop_gives_each_queued_write_a_single_attempt() {
    const PEERS: usize = 5;
    let store = Arc::new(ScriptedStore::default());
    store.set_down(true);
    let (mut pm, tasks) = server(&store);
    for id in peer_ids(PEERS) {
        accept(&mut pm, &id, UUID, PK1).await.unwrap();
    }
    eventually("breaker open", async || {
        store.attempt_count() == BREAKER_THRESHOLD as usize
    })
    .await;

    let start = Instant::now();
    tasks.shutdown().await;

    assert!(start.elapsed() < Duration::from_millis(100), "no backoff");
    assert_eq!(
        store.attempt_count(),
        BREAKER_THRESHOLD as usize + PEERS,
        "one attempt per queued write"
    );
    assert_eq!(pm.persist().lost_at_stop(), PEERS);
}

/// M-A: `reload` changes the cached peer in place or forgets a deleted id.
#[tokio::test]
async fn reload_replaces_cached_keys_in_place_or_forgets_the_id() {
    let store = Arc::new(ScriptedStore::default());
    store.inner.upsert_pk(&record(ID, UUID, PK1)).await.unwrap();
    store
        .inner
        .upsert_pk(&record("gone-peer", UUID, PK1))
        .await
        .unwrap();
    let (pm, tasks) = server(&store);
    pm.preload(SETTLE).await.unwrap();
    let cached = pm.get(ID).await.unwrap();
    // Changed and deleted out of band.
    store.inner.upsert_pk(&record(ID, UUID, PK2)).await.unwrap();
    store.deleted.lock().unwrap().push("gone-peer".to_owned());

    pm.reload(ID).await.unwrap();
    pm.reload("gone-peer").await.unwrap();

    assert!(
        Arc::ptr_eq(&pm.get(ID).await.unwrap(), &cached),
        "same entry"
    );
    assert_eq!(cached_pk(&pm, ID).await.as_deref(), Some(PK2));
    assert!(pm.get("gone-peer").await.is_none());
    tasks.shutdown().await;
}
