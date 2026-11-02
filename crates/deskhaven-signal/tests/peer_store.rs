//! M1-05: the injected `PeerStore` over the wire. Registrations are preloaded before the
//! server listens, the UDP / TCP paths never wait for the store afterwards, a store that
//! cannot be loaded fails `run`, a store outage keeps bound devices online, and failed writes
//! neither weaken the anti id-hijack check nor get lost.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use async_trait::async_trait;
use deskhaven_signal::{
    MemoryPeerStore, PeerRecord, PeerStore, PeerUpsert, SignalConfig, SignalError, run,
};
use hbb_common::{
    rendezvous_proto::register_pk_response::Result::{OK, SERVER_ERROR, UUID_MISMATCH},
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
    Conn, PeerKeys, Rendezvous, STEP, TestServer, allow_all, config, decode_addr, expect_msg,
    free_base_port, online_bits, ports_free,
};
use tokio_util::sync::CancellationToken;

/// After an outage the writer's breaker may still be open for up to its longest cool-down
/// (30 s); writing the backlog follows at once.
const RECOVERY: Duration = Duration::from_secs(40);
const POLL: Duration = Duration::from_millis(100);
/// How often a stalled upsert checks whether it may go on.
const STALL_POLL: Duration = Duration::from_millis(10);
/// The server's write queue bound for ids that are not bound yet (`persist.rs` `WRITE_QUEUE`).
const WRITE_QUEUE: usize = 1024;
/// RegisterPk per source IP in the queue test, below the per-IP limit (`check_ip_block`: 30
/// a minute).
const REGISTRATIONS_PER_IP: usize = 25;

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
    /// Upserts wait while set (and then go through).
    stall_upserts: AtomicBool,
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

    async fn stored_ip(&self, id: &str) -> Option<IpAddr> {
        self.inner.get(id).await.unwrap().and_then(|r| r.ip)
    }

    fn set_down(&self, down: bool) {
        let fail = if down { usize::MAX } else { 0 };
        self.fail_upserts.store(fail, Ordering::SeqCst);
    }
}

#[async_trait]
impl PeerStore for TestStore {
    async fn load_page(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<PeerRecord>, deskhaven_signal::StoreError> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        if self.hang_load.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        if self.fail_load.load(Ordering::SeqCst) {
            return Err(deskhaven_signal::StoreError::backend("store down"));
        }
        self.inner.load_page(after, limit).await
    }

    async fn get(&self, id: &str) -> Result<Option<PeerRecord>, deskhaven_signal::StoreError> {
        self.gets.fetch_add(1, Ordering::SeqCst);
        self.inner.get(id).await
    }

    async fn upsert_pk(&self, rec: &PeerUpsert) -> Result<(), deskhaven_signal::StoreError> {
        self.upserts.fetch_add(1, Ordering::SeqCst);
        while self.stall_upserts.load(Ordering::SeqCst) {
            sleep(STALL_POLL).await;
        }
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
    let res = timeout(
        STEP,
        run(config(port), store, allow_all(), CancellationToken::new()),
    )
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
    let res = timeout(STEP, run(cfg, store, allow_all(), CancellationToken::new()))
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

/// H-A: with the store down, a bound fleet stays online like upstream. Over two rounds, two
/// seconds apart (the writer's breaker opens meanwhile), every peer keeps refreshing over
/// RegisterPeer without being asked for its pk, moves to a new source address with a
/// RegisterPk that is
/// answered OK (never SERVER_ERROR), shows online in OnlineRequest and receives punch holes at
/// its new address. Once the store is back, every peer is saved with its latest IP, written
/// once (the moves were coalesced).
#[tokio::test]
async fn store_outage_keeps_bound_fleet_online_and_persists_after_recovery() {
    // Arrange
    const PEERS: usize = 20;
    let store = TestStore::shared();
    let server = start(&store).await;
    let ids: Vec<String> = (0..PEERS).map(|n| format!("fleet-peer-{n:02}")).collect();
    let mut fleet = Vec::new();
    for id in &ids {
        fleet.push(server.online_peer(id).await);
    }
    eventually("fleet saved", STEP, async || {
        store.upserts.load(Ordering::SeqCst) == PEERS
    })
    .await;

    // Act + Assert
    store.set_down(true);
    for round in 0..2 {
        for (id, (client, keys)) in ids.iter().zip(fleet.iter_mut()) {
            assert!(!client.register_peer(id).await.request_pk, "{id}: refresh");
            let mut moved = server.udp().await;
            assert_eq!(moved.register_pk(id, keys).await, OK, "{id}: round {round}");
            assert!(!moved.register_peer(id).await.request_pk, "{id}: moved");
            *client = moved;
        }
        sleep(Duration::from_secs(2)).await;
    }
    let names: Vec<&str> = ids.iter().map(String::as_str).collect();
    let online = server.nat().await.online_request(&names).await;
    assert_eq!(
        online_bits(&online.states, PEERS),
        vec![true; PEERS],
        "all online"
    );
    for (id, (client, _)) in ids.iter().zip(fleet.iter_mut()) {
        let mut controller = server.tcp().await;
        controller.punch_hole_request(server.punch_req(id)).await;
        let ph = expect_msg!(client.recv().await, PunchHole);
        assert_eq!(
            decode_addr(&ph.socket_addr),
            controller.local_addr(),
            "{id}"
        );
    }
    let before = store.upserts.load(Ordering::SeqCst);
    store.set_down(false);
    eventually("backlog saved", RECOVERY, async || {
        let mut all = true;
        for (id, (client, _)) in ids.iter().zip(&fleet) {
            all &= store.stored_ip(id).await == Some(client.local_addr().ip());
        }
        all
    })
    .await;
    sleep(POLL).await;
    let recovery = store.upserts.load(Ordering::SeqCst) - before;
    assert_eq!(recovery, PEERS, "each peer written once");
    server.stop().await;
}

/// M-A: the store binds an id to another device (changed out of band after the preload) and
/// answers the write of a freshly accepted registration with UuidMismatch. The store's
/// binding is loaded back into memory (the writer re-reads it with `get`): right after that,
/// an attacker with yet another uuid gets UUID_MISMATCH (eviction let the first comer claim the
/// id), so does the device memory had accepted, and the store's owner is OK.
#[tokio::test]
async fn store_uuid_mismatch_rebinds_the_store_owner_in_memory() {
    // Arrange
    const ID: &str = "contested-peer";
    let store = TestStore::shared();
    let server = start(&store).await;
    let owner = PeerKeys::random();
    store.seed(ID, &owner).await;
    let (mut accepted, accepted_keys) = server.online_peer(ID).await;
    eventually("mismatch handled", STEP, async || {
        store.gets.load(Ordering::SeqCst) == 1
    })
    .await;
    sleep(POLL).await;

    // Act
    let attacker = server
        .udp()
        .await
        .register_pk(ID, &PeerKeys::random())
        .await;
    let previously_accepted = accepted.register_pk(ID, &accepted_keys).await;
    let store_owner = server.udp().await.register_pk(ID, &owner).await;

    // Assert
    assert_eq!(attacker, UUID_MISMATCH, "yet another uuid");
    assert_eq!(
        previously_accepted, UUID_MISMATCH,
        "the binding the store refused"
    );
    assert_eq!(store_owner, OK, "the store's owner");
    assert_eq!(store.stored_uuid(ID).await, Some(owner.uuid.to_vec()));
    server.stop().await;
}

/// M1-08 (07 §2.4): upstream answered RegisterPk OK whatever `update_pk` returned (rs:466).
/// With the write-behind (M1-05) the one failure left on that path is a full write queue
/// refusing an id that is not bound yet: it is answered SERVER_ERROR (the client retries), over
/// UDP and over TCP alike, and nothing is registered; bound peers are still accepted.
#[tokio::test]
async fn full_write_queue_answers_server_error_over_udp_and_tcp() {
    // Arrange: the store stalls, so accepted registrations of new ids pile up in the queue.
    let store = TestStore::shared();
    store.stall_upserts.store(true, Ordering::SeqCst);
    let server = start(&store).await;
    let first = PeerKeys::random();
    let mut udp = server.udp().await;
    assert_eq!(udp.register_pk("queued-0", &first).await, OK);

    // Act: new ids until one is refused.
    let mut refused_at = None;
    for n in 1..=WRITE_QUEUE + REGISTRATIONS_PER_IP {
        if n % REGISTRATIONS_PER_IP == 0 {
            udp = server.udp().await;
        }
        let res = udp
            .register_pk(&format!("queued-{n}"), &PeerKeys::random())
            .await;
        if res == SERVER_ERROR {
            refused_at = Some(n);
            break;
        }
        assert_eq!(res, OK, "queued-{n}");
    }
    let tcp = server
        .tcp()
        .await
        .register_pk("queued-over-tcp", &PeerKeys::random())
        .await;
    let bound = server.udp().await.register_pk("queued-0", &first).await;

    // Assert
    let refused_at = refused_at.expect("the write queue never filled up");
    assert!(refused_at >= WRITE_QUEUE, "refused after {refused_at}");
    assert_eq!(tcp, SERVER_ERROR, "over TCP");
    assert_eq!(bound, OK, "a bound peer");
    let online = server
        .nat()
        .await
        .online_request(&["queued-over-tcp"])
        .await;
    assert_eq!(online_bits(&online.states, 1), [false], "not registered");
    store.stall_upserts.store(false, Ordering::SeqCst);
    server.stop().await;
}
