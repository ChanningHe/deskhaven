//! M1-05: the injected `PeerStore` over the wire. Registrations are preloaded before the
//! server listens, the UDP / TCP paths never wait for the store afterwards, a store that
//! cannot be loaded fails `run`, and failed writes neither weaken the anti id-hijack check nor
//! get lost.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use async_trait::async_trait;
use deskhaven_signal::{
    MemoryPeerStore, PeerRecord, PeerStore, PeerUpsert, SignalConfig, SignalError, run,
};
use hbb_common::{
    rendezvous_proto::register_pk_response::Result::{OK, UUID_MISMATCH},
    tokio::{
        self,
        time::{Duration, Instant, sleep, timeout},
    },
};
use std::{
    net::{IpAddr, Ipv4Addr},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::SystemTime,
};
use support::{
    Conn, PeerKeys, Rendezvous, STEP, TestServer, config, expect_msg, free_base_port, ports_free,
};
use tokio_util::sync::CancellationToken;

/// Exhausting the writer's retries takes 0.25 + 1 + 2 + 4 s; the client's next RegisterPeer
/// then gets `request_pk` again.
const RETRIES_EXHAUSTED: Duration = Duration::from_secs(15);
const POLL: Duration = Duration::from_millis(100);

/// A memory store that counts calls and can fail or hang.
#[derive(Default)]
struct TestStore {
    inner: MemoryPeerStore,
    loads: AtomicUsize,
    gets: AtomicUsize,
    upserts: AtomicUsize,
    /// Upcoming upserts that fail with a backend error.
    fail_upserts: AtomicUsize,
    fail_load: AtomicBool,
    hang_load: AtomicBool,
}

impl TestStore {
    fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    async fn seed(&self, id: &str, keys: &PeerKeys) {
        let rec = PeerUpsert {
            id: id.to_owned(),
            uuid: keys.uuid.to_vec(),
            pk: keys.pk.to_vec(),
            ip: IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
            registered_at: SystemTime::now(),
        };
        self.inner.upsert_pk(&rec).await.unwrap();
    }

    async fn stored_uuid(&self, id: &str) -> Option<Vec<u8>> {
        self.inner.get(id).await.unwrap().map(|r| r.uuid)
    }
}

#[async_trait]
impl PeerStore for TestStore {
    async fn load_all(&self) -> Result<Vec<PeerRecord>, deskhaven_signal::StoreError> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        if self.hang_load.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        if self.fail_load.load(Ordering::SeqCst) {
            return Err(deskhaven_signal::StoreError::backend("store down"));
        }
        self.inner.load_all().await
    }

    async fn get(&self, id: &str) -> Result<Option<PeerRecord>, deskhaven_signal::StoreError> {
        self.gets.fetch_add(1, Ordering::SeqCst);
        self.inner.get(id).await
    }

    async fn upsert_pk(&self, rec: &PeerUpsert) -> Result<(), deskhaven_signal::StoreError> {
        self.upserts.fetch_add(1, Ordering::SeqCst);
        // Upserts come from one writer task, one at a time: load-then-sub cannot race.
        if self.fail_upserts.load(Ordering::SeqCst) > 0 {
            self.fail_upserts.fetch_sub(1, Ordering::SeqCst);
            return Err(deskhaven_signal::StoreError::backend("injected failure"));
        }
        self.inner.upsert_pk(rec).await
    }
}

async fn start(store: &Arc<TestStore>) -> TestServer {
    TestServer::start_with_store(|cfg| cfg, store.clone()).await
}

async fn eventually(what: &str, wait: Duration, mut done: impl AsyncFnMut() -> bool) {
    let deadline = Instant::now() + wait;
    while !done().await {
        assert!(Instant::now() < deadline, "never happened: {what}");
        sleep(POLL).await;
    }
}

/// Every stored registration is enforced from the first packet: another uuid gets
/// UUID_MISMATCH, while the owner (same uuid and pk, any IP) is OK and not asked for its pk.
#[tokio::test]
async fn preloaded_registrations_are_enforced_from_the_first_packet() {
    // Arrange
    const DEVICES: usize = 200;
    let store = TestStore::shared();
    let mut devices = Vec::new();
    for n in 0..DEVICES {
        let (id, keys) = (format!("preloaded-{n}"), PeerKeys::random());
        store.seed(&id, &keys).await;
        devices.push((id, keys));
    }
    let server = start(&store).await;

    // Act + Assert (a sample; each check uses fresh source IPs)
    for (id, keys) in devices.iter().step_by(DEVICES / 10) {
        let thief = server
            .udp()
            .await
            .register_pk(id, &PeerKeys::random())
            .await;
        assert_eq!(thief, UUID_MISMATCH, "{id}: another uuid");
        let mut owner = server.udp().await;
        assert!(!owner.register_peer(id).await.request_pk, "{id}: pk known");
        assert_eq!(owner.register_pk(id, keys).await, OK, "{id}: owner");
    }
    assert_eq!(store.loads.load(Ordering::SeqCst), 1);
    server.stop().await;
}

/// After the preload nothing on the UDP / TCP paths reads the store: registrations of known
/// and unknown ids, hijack attempts and punch holes to unknown ids only ever write.
#[tokio::test]
async fn hot_path_never_reads_the_store_after_preload() {
    // Arrange
    let store = TestStore::shared();
    let known = PeerKeys::random();
    store.seed("known-peer", &known).await;
    let server = start(&store).await;

    // Act
    let mut fresh = server.udp().await;
    assert!(fresh.register_peer("fresh-peer").await.request_pk);
    assert_eq!(
        fresh.register_pk("fresh-peer", &PeerKeys::random()).await,
        OK
    );
    let thief = server
        .udp()
        .await
        .register_pk("known-peer", &PeerKeys::random())
        .await;
    for n in 0..20 {
        let mut controller = server.tcp().await;
        controller
            .punch_hole_request(server.punch_req(&format!("random-{n}")))
            .await;
        expect_msg!(controller.recv().await, PunchHoleResponse);
    }
    let mut known_peer = server.udp().await;
    assert!(!known_peer.register_peer("known-peer").await.request_pk);
    eventually("the one write lands", STEP, async || {
        store.stored_uuid("fresh-peer").await.is_some()
    })
    .await;
    server.stop().await;

    // Assert
    assert_eq!(thief, UUID_MISMATCH);
    assert_eq!(store.loads.load(Ordering::SeqCst), 1, "one preload");
    assert_eq!(store.gets.load(Ordering::SeqCst), 0, "no lookups");
    assert_eq!(
        store.upserts.load(Ordering::SeqCst),
        1,
        "one write (fresh-peer)"
    );
}

/// A store that cannot be loaded fails `run` before anything is bound (the embedding process
/// supervises and restarts).
#[tokio::test]
async fn preload_failure_fails_run_before_binding() {
    // Arrange
    let store = TestStore::shared();
    store.fail_load.store(true, Ordering::SeqCst);
    let port = free_base_port();

    // Act
    let res = timeout(STEP, run(config(port), store, CancellationToken::new()))
        .await
        .expect("run returns promptly");

    // Assert
    assert!(matches!(res, Err(SignalError::PeerPreload(_))), "{res:?}");
    assert!(ports_free(port), "nothing was bound");
}

#[tokio::test]
async fn preload_timeout_fails_run() {
    // Arrange
    let store = TestStore::shared();
    store.hang_load.store(true, Ordering::SeqCst);
    let cfg = SignalConfig {
        peer_preload_timeout: Duration::from_millis(200),
        ..config(free_base_port())
    };

    // Act
    let res = timeout(STEP, run(cfg, store, CancellationToken::new()))
        .await
        .expect("run returns once the preload times out");

    // Assert
    assert!(matches!(res, Err(SignalError::PeerPreload(_))), "{res:?}");
}

/// Scenario a: the owner's first registration is answered OK while its write keeps failing
/// for a while; the accepted entry stays in memory, so another uuid still gets UUID_MISMATCH,
/// and the retried write eventually stores the owner's uuid.
#[tokio::test]
async fn transient_write_failure_still_rejects_another_uuid() {
    // Arrange
    let store = TestStore::shared();
    store.fail_upserts.store(2, Ordering::SeqCst);
    let server = start(&store).await;
    let (_owner, owner_keys) = server.online_peer("owner-peer").await;

    // Act
    let thief = server
        .udp()
        .await
        .register_pk("owner-peer", &PeerKeys::random())
        .await;

    // Assert
    assert_eq!(thief, UUID_MISMATCH);
    eventually("the retried write lands", STEP, async || {
        store.stored_uuid("owner-peer").await.is_some()
    })
    .await;
    assert_eq!(
        store.stored_uuid("owner-peer").await,
        Some(owner_keys.uuid.to_vec())
    );
    server.stop().await;
}

/// A write that fails on every retry is not dropped: the owner's next RegisterPeer asks for
/// its pk again, and the RegisterPk it sends re-queues the write, which lands once the store
/// recovers.
#[tokio::test]
async fn unsaved_registration_is_requested_again_and_saved() {
    // Arrange: every attempt of the first write fails (1 + 4 retries).
    let store = TestStore::shared();
    store.fail_upserts.store(5, Ordering::SeqCst);
    let server = start(&store).await;
    let (mut owner, keys) = server.online_peer("unsaved-peer").await;

    // Act
    eventually("pk requested again", RETRIES_EXHAUSTED, async || {
        owner.register_peer("unsaved-peer").await.request_pk
    })
    .await;
    let again = owner.register_pk("unsaved-peer", &keys).await;

    // Assert
    assert_eq!(again, OK);
    eventually("the re-queued write lands", STEP, async || {
        store.stored_uuid("unsaved-peer").await.is_some()
    })
    .await;
    assert!(!owner.register_peer("unsaved-peer").await.request_pk);
    owner.assert_silent().await;
    server.stop().await;
}
