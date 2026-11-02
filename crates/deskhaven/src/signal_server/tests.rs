//! Signal server wiring: key conversion, relay settings, the settings -> server sync on a real
//! server (also on connections that are already open), startup gate and supervised restarts.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use deskhaven_config::bootstrap::Security;
use deskhaven_config::settings::{self, Change, PinSet, Snapshot};
use deskhaven_core::crypto::{MasterKey, SecretBox};
use deskhaven_core::devices::DeviceRepository;
use deskhaven_core::keys::SignalKeyPair;
use deskhaven_core::settings::{SettingsService, UpdateRequest};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_signal::{
    AllowAll, ControlPlane, Fault, MemoryPeerStore, PeerRecord, PeerStore, PeerUpsert,
    SignalConfig, StoreError, WsHeaders,
};
use hbb_common::bytes_codec::BytesCodec;
use hbb_common::futures_util::SinkExt as _;
use hbb_common::protobuf::Message as _;
use hbb_common::rendezvous_proto::{
    NatType, PunchHole, PunchHoleRequest, RegisterPeer, RegisterPk, RendezvousMessage,
    register_pk_response, rendezvous_message,
};
use hbb_common::tokio_util::codec::Framed;
use serde_json::{Value as Json, json};
use sodiumoxide::crypto::sign;
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::watch;
use tokio::time::{Instant, sleep, timeout};
use tokio_util::sync::CancellationToken;

use super::relay_sync::is_new_rejection;
use super::supervisor::next_delay;
use super::{Backoff, Launcher, RunningSignal, SignalSettings, base_config, secret_key};
use crate::http::signal_ready;
use crate::signal_store::SignalPeerStore;

const LOCALHOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
/// The target registers from another loopback address than the controller connects from:
/// the same source IP would make the server answer FetchLocalAddr instead of PunchHole
/// (Linux routes all of 127.0.0.0/8 to `lo`).
pub(super) const TARGET_IP: IpAddr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));
pub(super) const STEP: Duration = Duration::from_secs(5);
pub(super) const POLL: Duration = Duration::from_millis(20);
const START_ATTEMPTS: usize = 10;
/// TEST-NET-1 (RFC 5737): resolve without DNS, never pass a health probe.
const RELAY_1: &str = "192.0.2.1:21117";
const RELAY_2: &str = "192.0.2.2:21117";
pub(super) const TARGET: &str = "123456789";
pub(super) const FAST: Backoff = Backoff {
    initial: Duration::from_millis(50),
    max: Duration::from_millis(200),
};

// --- configuration -------------------------------------------------------------------------

/// M1-08 review (HIGH-1): the websocket port honours `X-Forwarded-For` only from
/// `security.trusted_proxies`, with the HTTP API's rule; with none configured (the default) the
/// client is always the socket peer.
#[test]
fn websocket_proxy_headers_follow_security_trusted_proxies() {
    // Arrange
    let pair = SignalKeyPair::generate().unwrap();
    let security = Security {
        trusted_proxies: vec!["10.0.0.0/8".parse().unwrap()],
        ..Default::default()
    };
    let mut headers = WsHeaders::new();
    headers.insert("x-forwarded-for", "203.0.113.5".parse().unwrap());
    headers.insert("x-real-ip", "198.51.100.7".parse().unwrap());
    let ip = |s: &str| s.parse::<IpAddr>().unwrap();

    // Act
    let trusting = base_config(&Default::default(), &security, &pair).unwrap();
    let default = base_config(&Default::default(), &Default::default(), &pair).unwrap();

    // Assert
    let resolve = |cfg: &SignalConfig, peer: &str| cfg.ws_client_ip.client_ip(ip(peer), &headers);
    assert_eq!(
        resolve(&trusting, "10.1.2.3"),
        ip("203.0.113.5"),
        "trusted proxy"
    );
    assert_eq!(
        resolve(&trusting, "11.0.0.1"),
        ip("11.0.0.1"),
        "untrusted peer"
    );
    assert_eq!(
        resolve(&default, "10.1.2.3"),
        ip("10.1.2.3"),
        "default trusts none"
    );
}

#[test]
fn signal_key_converts_to_the_same_sodiumoxide_key() {
    // Arrange
    let pair = SignalKeyPair::generate().unwrap();
    let message = b"IdPk to sign";

    // Act
    let sk = secret_key(&pair).unwrap();
    let cfg = base_config(&Default::default(), &Default::default(), &pair).unwrap();

    // Assert
    assert_eq!(sk.public_key().0, pair.public_key());
    assert_eq!(
        cfg.licence_key(),
        pair.public_key_base64(),
        "advertised key"
    );
    assert_eq!(
        sign::sign_detached(message, &sk).to_bytes(),
        pair.sign(message)
    );
    assert_eq!(cfg.port, 21116);
    assert_eq!(cfg.bind, None, "all interfaces");
}

#[test]
fn signal_settings_follow_the_snapshot() {
    // Arrange
    let defaults = Snapshot::defaults();
    let changed = with(&[
        (
            "signal.relay_servers",
            json!([RELAY_1, "relay.example.com:21117"]),
        ),
        ("signal.always_use_relay", json!(true)),
        ("signal.require_kx_v1", json!(true)),
    ]);

    // Act
    let (before, after) = (
        SignalSettings::from_snapshot(&defaults),
        SignalSettings::from_snapshot(&changed),
    );

    // Assert
    assert_eq!(before, SignalSettings::default());
    assert_eq!(after.servers, [RELAY_1, "relay.example.com:21117"]);
    assert!(after.always_use_relay && after.require_kx_v1);
    let cfg = after.apply_to(&SignalConfig::new(sign::gen_keypair().1));
    assert_eq!(cfg.relay_servers, after.servers);
    assert!(cfg.always_use_relay && cfg.require_kx_v1);
}

/// `Snapshot::defaults()` with the given settings applied.
pub(super) fn with(changes: &[(&str, Json)]) -> Snapshot {
    let changes: Vec<Change> = changes
        .iter()
        .map(|(key, value)| Change {
            key: (*key).to_owned(),
            value: value.clone(),
            version: 0,
        })
        .collect();
    let accepted = settings::prepare(&PinSet::empty(), &changes).unwrap();
    Snapshot::defaults().with_accepted(&accepted, None, 0)
}

// --- settings -> running server ------------------------------------------------------------

/// Settings changed through `SettingsService` reach the running server: subsequent punch
/// holes, also on a controller connection opened before the change, name the new relay, and
/// `always_use_relay` forces relayed connections. Registrations land in `devices`.
#[tokio::test]
async fn settings_changes_reach_new_and_open_connections() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let service = settings_service(db).await;
        update(&service, "signal.relay_servers", json!([RELAY_1]), 0).await;
        let store: Arc<dyn PeerStore> =
            Arc::new(SignalPeerStore::new(DeviceRepository::new(db.conn.clone())));
        let token = CancellationToken::new();
        let (running, port, key) = start_on_free_port(&service, store, &token).await;
        let mut target = Target::register(port).await;
        let mut controller = Controller::connect(port, key).await;
        wait_punch(&mut controller, &mut target, |ph| {
            ph.relay_server == RELAY_1
        })
        .await;

        // Act
        update(&service, "signal.relay_servers", json!([RELAY_2]), 1).await;
        let relayed = wait_punch(&mut controller, &mut target, |ph| {
            ph.relay_server == RELAY_2
        });
        relayed.await;
        update(&service, "signal.always_use_relay", json!(true), 0).await;
        let forced = wait_punch(&mut controller, &mut target, |ph| {
            ph.nat_type.enum_value() == Ok(NatType::SYMMETRIC)
        });
        let forced = forced.await;

        // Assert
        assert_eq!(forced.relay_server, RELAY_2, "{}", db.backend);
        token.cancel();
        timeout(STEP, running.task).await.unwrap().unwrap();
        let device = DeviceRepository::new(db.conn.clone())
            .find_by_rustdesk_id(TARGET)
            .await
            .unwrap();
        assert!(device.is_some(), "{}: devices row persisted", db.backend);
    })
    .await;
}

/// A relay list that does not resolve is rejected as a whole; the server keeps the previous
/// list and the next valid change applies.
#[tokio::test]
async fn rejected_relay_list_keeps_the_previous_one() {
    // Arrange
    let (tx, rx) = watch::channel(Arc::new(with(&[(
        "signal.relay_servers",
        json!([RELAY_1]),
    )])));
    let token = CancellationToken::new();
    let (running, port, key) = start_launcher(rx, memory_store(), &token).await;
    let mut target = Target::register(port).await;
    let mut controller = Controller::connect(port, key).await;
    wait_punch(&mut controller, &mut target, |ph| {
        ph.relay_server == RELAY_1
    })
    .await;

    // Act
    tx.send_replace(Arc::new(with(&[(
        "signal.relay_servers",
        json!(["relay.invalid:21117"]),
    )])));
    sleep(super::relay_sync::DEBOUNCE * 3).await;
    let kept = punch(&mut controller, &mut target).await;
    tx.send_replace(Arc::new(with(&[(
        "signal.relay_servers",
        json!([RELAY_2]),
    )])));

    // Assert
    assert_eq!(kept.relay_server, RELAY_1, "previous list kept");
    wait_punch(&mut controller, &mut target, |ph| {
        ph.relay_server == RELAY_2
    })
    .await;
    token.cancel();
    timeout(STEP, running.task).await.unwrap().unwrap();
}

// --- startup gate and supervision ----------------------------------------------------------

#[tokio::test]
async fn first_attempt_failure_fails_startup() {
    // Arrange
    let store = Arc::new(FlakyStore::failing(usize::MAX));
    let token = CancellationToken::new();
    let launcher = Launcher::new(
        local_config(free_base_port()),
        store,
        allow_all(),
        defaults(),
    );

    // Act
    let res = timeout(STEP, super::start(launcher, FAST, &token))
        .await
        .unwrap();

    // Assert
    let err = format!("{:#}", res.err().unwrap());
    assert!(err.contains("signal server failed to start"), "{err}");
    assert!(err.contains("cannot load the peer registry"), "{err}");
}

#[tokio::test]
async fn taken_port_fails_startup_naming_the_port() {
    // Arrange
    let port = free_base_port();
    let _taken = std::net::TcpListener::bind((LOCALHOST, port + 2)).unwrap();
    let token = CancellationToken::new();
    let launcher = Launcher::new(local_config(port), memory_store(), allow_all(), defaults());

    // Act
    let res = timeout(STEP, super::start(launcher, FAST, &token))
        .await
        .unwrap();

    // Assert
    let err = format!("{:#}", res.err().unwrap());
    assert!(
        err.contains(&format!("cannot listen on tcp port {}", port + 2)),
        "{err}"
    );
}

/// A server that fails (here: its peer registry cannot be loaded) is restarted with growing
/// delays until it serves; readiness follows.
#[tokio::test]
async fn failed_server_is_restarted_with_backoff_until_it_serves() {
    // Arrange
    const FAILURES: usize = 3;
    let store = Arc::new(FlakyStore::failing(FAILURES));
    let token = CancellationToken::new();
    let launcher = Launcher::new(
        local_config(free_base_port()),
        store.clone(),
        allow_all(),
        defaults(),
    );

    // Act
    let running = super::spawn_supervised(launcher, FAST, &token);
    assert!(
        signal_ready(Some(&running.status)).is_err(),
        "not ready while failing"
    );
    wait_until(|| running.status.is_serving()).await;

    // Assert
    assert!(signal_ready(Some(&running.status)).is_ok());
    let attempts = store.attempts();
    assert_eq!(attempts.len(), FAILURES + 1, "{attempts:?}");
    let gaps: Vec<Duration> = attempts.windows(2).map(|w| w[1] - w[0]).collect();
    let expected = [FAST.initial, FAST.initial * 2, FAST.max];
    for (gap, min) in gaps.iter().zip(expected) {
        assert!(*gap >= min, "gaps {gaps:?} below {expected:?}");
    }
    token.cancel();
    timeout(STEP, running.task).await.unwrap().unwrap();
}

#[tokio::test]
async fn shutdown_during_backoff_stops_the_supervisor_promptly() {
    // Arrange
    let store = Arc::new(FlakyStore::failing(usize::MAX));
    let token = CancellationToken::new();
    let slow = Backoff {
        initial: Duration::from_secs(60),
        max: Duration::from_secs(60),
    };
    let launcher = Launcher::new(
        local_config(free_base_port()),
        store.clone(),
        allow_all(),
        defaults(),
    );
    let running = super::spawn_supervised(launcher, slow, &token);
    wait_until(|| !store.attempts().is_empty()).await;

    // Act
    token.cancel();

    // Assert
    timeout(STEP, running.task).await.unwrap().unwrap();
    assert_eq!(store.attempts().len(), 1, "no restart after shutdown");
}

#[test]
fn next_delay_doubles_up_to_max_and_resets_after_a_long_run() {
    let b = Backoff::DEFAULT;
    let secs = Duration::from_secs;
    let quick = Duration::from_millis(10);

    assert_eq!(next_delay(b, None, quick), secs(1), "first failure");
    assert_eq!(next_delay(b, None, secs(3600)), secs(1));
    let mut delay = None;
    let waited: Vec<Duration> = (0..8)
        .map(|_| {
            let next = next_delay(b, delay, quick);
            delay = Some(next);
            next
        })
        .collect();
    let expected = [1, 2, 4, 8, 16, 32, 60, 60].map(secs);
    assert_eq!(waited, expected, "doubling, capped at max");
    assert_eq!(
        next_delay(b, Some(secs(60)), secs(59)),
        secs(60),
        "short of max"
    );
    assert_eq!(
        next_delay(b, Some(secs(60)), secs(60)),
        secs(1),
        "ran max: reset"
    );
    assert_eq!(next_delay(b, Some(secs(8)), secs(61)), secs(1));
}

/// M1-06 acceptance, real path: a server that served longer than `max` and then fails is
/// restarted after the initial delay, even when earlier quick failures had raised it.
#[tokio::test]
async fn long_running_server_failure_restarts_after_the_initial_delay() {
    // Arrange
    let backoff = REAL_PATH;
    let store = Arc::new(FlakyStore::failing(0));
    let token = CancellationToken::new();
    let (running, _, _) =
        start_launcher_with(defaults(), store.clone(), allow_all(), backoff, &token).await;
    let first = fail_and_measure_restart(&running, &store).await;
    let second = fail_and_measure_restart(&running, &store).await;

    // Act: serve past `max`, then fail.
    sleep(backoff.max + backoff.initial).await;
    let after_long_run = fail_and_measure_restart(&running, &store).await;

    // Assert
    assert!(first >= backoff.initial, "{first:?}");
    assert!(second >= backoff.initial * 2, "raised: {second:?}");
    assert!(
        after_long_run >= backoff.initial && after_long_run < backoff.initial * 3,
        "reset to the initial delay, got {after_long_run:?}"
    );
    token.cancel();
    timeout(STEP, running.task).await.unwrap().unwrap();
}

/// A server that keeps failing shortly after serving waits longer before each restart.
#[tokio::test]
async fn quickly_failing_server_backs_off_further() {
    // Arrange
    let backoff = REAL_PATH;
    let store = Arc::new(FlakyStore::failing(0));
    let token = CancellationToken::new();
    let (running, _, _) =
        start_launcher_with(defaults(), store.clone(), allow_all(), backoff, &token).await;

    // Act
    let mut gaps = Vec::new();
    for _ in 0..3 {
        gaps.push(fail_and_measure_restart(&running, &store).await);
    }

    // Assert
    let minimum = [backoff.initial, backoff.initial * 2, backoff.max];
    for (gap, min) in gaps.iter().zip(minimum) {
        assert!(*gap >= min, "gaps {gaps:?} below {minimum:?}");
    }
    token.cancel();
    timeout(STEP, running.task).await.unwrap().unwrap();
}

/// M2: when the supervisor is gone (here aborted, as a panic would drop it), its server is
/// detached, not stopped; `SignalStatus::stopped` (what `serve` awaits before closing the
/// database) still waits until that server drained its in-flight registration write.
#[tokio::test]
async fn status_stopped_waits_for_a_detached_server_to_drain() {
    // Arrange
    let store = Arc::new(SlowStore::default());
    let token = CancellationToken::new();
    let (running, port, _) = start_launcher(defaults(), store.clone(), &token).await;
    Target::register(port).await;
    wait_until(|| store.started.load(Ordering::SeqCst) == 1).await;
    running.task.abort();
    assert!(running.task.await.unwrap_err().is_cancelled());

    // Act
    token.cancel();
    timeout(STEP, running.status.stopped()).await.unwrap();

    // Assert
    assert_eq!(store.finished.load(Ordering::SeqCst), 1, "drained first");
    assert!(!running.status.is_serving());
}

#[test]
fn rejected_relay_list_warns_once_per_distinct_list() {
    let a = vec!["a.invalid:21117".to_owned()];
    let b = vec!["b.invalid:21117".to_owned()];
    assert!(is_new_rejection(None, &a));
    assert!(!is_new_rejection(Some(&a), &a), "retry of the same list");
    assert!(is_new_rejection(Some(&a), &b));
}

// --- helpers -------------------------------------------------------------------------------

/// Small enough for tests, far enough apart to tell reset (initial) from growth.
const REAL_PATH: Backoff = Backoff {
    initial: Duration::from_millis(100),
    max: Duration::from_millis(400),
};

/// Makes the current (serving) server fail and returns the time until the next launch.
async fn fail_and_measure_restart(running: &RunningSignal, store: &FlakyStore) -> Duration {
    wait_until(|| running.status.is_serving()).await;
    let launches = store.attempts().len();
    let failed_at = Instant::now();
    running.status.handle().inject_fault(Fault::Error).unwrap();
    wait_until(|| store.attempts().len() > launches).await;
    store.attempts()[launches] - failed_at
}

/// Every write takes `SLOW_WRITE`; counts writes started and finished.
#[derive(Default)]
struct SlowStore {
    started: AtomicUsize,
    finished: AtomicUsize,
}

const SLOW_WRITE: Duration = Duration::from_millis(1000);

#[async_trait]
impl PeerStore for SlowStore {
    async fn load_page(&self, _: Option<&str>, _: usize) -> Result<Vec<PeerRecord>, StoreError> {
        Ok(Vec::new())
    }

    async fn get(&self, _: &str) -> Result<Option<PeerRecord>, StoreError> {
        Ok(None)
    }

    async fn upsert_pk(&self, _: &PeerUpsert) -> Result<(), StoreError> {
        self.started.fetch_add(1, Ordering::SeqCst);
        sleep(SLOW_WRITE).await;
        self.finished.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn crypto() -> Arc<SecretBox> {
    Arc::new(SecretBox::new(&MasterKey::from_bytes(&[7; 32])))
}

async fn settings_service(db: &TestDb) -> SettingsService {
    let (service, _) = SettingsService::load(db.conn.clone(), crypto(), PinSet::empty())
        .await
        .unwrap();
    service
}

async fn update(service: &SettingsService, key: &str, value: Json, version: i64) {
    let request = UpdateRequest {
        changes: vec![Change {
            key: key.to_owned(),
            value,
            version,
        }],
        actor: None,
        reason: None,
    };
    service.update(request).await.unwrap();
}

fn defaults() -> watch::Receiver<Arc<Snapshot>> {
    // The sender stays alive (leaked) so the receiver never reports a closed channel.
    let (tx, rx) = watch::channel(Arc::new(Snapshot::defaults()));
    std::mem::forget(tx);
    rx
}

pub(super) fn memory_store() -> Arc<dyn PeerStore> {
    Arc::new(MemoryPeerStore::new())
}

fn allow_all() -> Arc<dyn ControlPlane> {
    Arc::new(AllowAll::new())
}

fn local_config(port: u16) -> SignalConfig {
    SignalConfig {
        bind: Some(LOCALHOST),
        port,
        ..SignalConfig::new(sign::gen_keypair().1)
    }
}

/// A base port whose TCP port-1, port, port+2 and UDP port were free a moment ago.
fn free_base_port() -> u16 {
    loop {
        let probe = std::net::TcpListener::bind((LOCALHOST, 0)).unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        if !(3..=u16::MAX - 2).contains(&port) {
            continue;
        }
        let free = [port - 1, port, port + 2]
            .iter()
            .all(|p| std::net::TcpListener::bind((LOCALHOST, *p)).is_ok())
            && std::net::UdpSocket::bind((LOCALHOST, port)).is_ok();
        if free {
            return port;
        }
    }
}

async fn start_on_free_port(
    service: &SettingsService,
    store: Arc<dyn PeerStore>,
    token: &CancellationToken,
) -> (RunningSignal, u16, String) {
    start_launcher(service.subscribe(), store, token).await
}

/// Starts on fresh free ports, retrying on a lost port race; also returns the licence key.
pub(super) async fn start_launcher(
    settings: watch::Receiver<Arc<Snapshot>>,
    store: Arc<dyn PeerStore>,
    token: &CancellationToken,
) -> (RunningSignal, u16, String) {
    start_launcher_with(settings, store, allow_all(), FAST, token).await
}

/// [`start_launcher`] with the given control plane and backoff.
pub(super) async fn start_launcher_with(
    settings: watch::Receiver<Arc<Snapshot>>,
    store: Arc<dyn PeerStore>,
    control: Arc<dyn ControlPlane>,
    backoff: Backoff,
    token: &CancellationToken,
) -> (RunningSignal, u16, String) {
    for _ in 0..START_ATTEMPTS {
        let port = free_base_port();
        let cfg = local_config(port);
        let key = cfg.licence_key();
        let launcher = Launcher::new(
            cfg,
            Arc::clone(&store),
            Arc::clone(&control),
            settings.clone(),
        );
        match timeout(STEP, super::start(launcher, backoff, token))
            .await
            .unwrap()
        {
            Ok(running) => return (running, port, key),
            Err(err) if format!("{err:#}").contains("cannot listen") => continue,
            Err(err) => panic!("{err:#}"),
        }
    }
    panic!("could not start the signal server");
}

pub(super) async fn wait_until(cond: impl Fn() -> bool) {
    let deadline = Instant::now() + STEP;
    while !cond() {
        assert!(Instant::now() < deadline, "condition never held");
        sleep(POLL).await;
    }
}

fn rv(set: impl FnOnce(&mut RendezvousMessage)) -> RendezvousMessage {
    let mut msg = RendezvousMessage::new();
    set(&mut msg);
    msg
}

/// A controlled peer: registered over UDP, receives the forwarded PunchHoles.
pub(super) struct Target(UdpSocket);

impl Target {
    pub(super) async fn register(port: u16) -> Self {
        let socket = UdpSocket::bind((TARGET_IP, 0)).await.unwrap();
        socket.connect((LOCALHOST, port)).await.unwrap();
        let mut target = Self(socket);
        let msg = rv(|m| {
            m.set_register_pk(RegisterPk {
                id: TARGET.to_owned(),
                uuid: b"target-uuid".to_vec().into(),
                pk: vec![9; 32].into(),
                ..Default::default()
            })
        });
        target.send(&msg).await;
        let res = target.recv().await;
        match res.union {
            Some(rendezvous_message::Union::RegisterPkResponse(r)) => {
                assert_eq!(r.result.enum_value(), Ok(register_pk_response::Result::OK));
            }
            other => panic!("expected RegisterPkResponse, got {other:?}"),
        }
        target
    }

    /// Sends a RegisterPeer (the UDP keep-alive); `request_pk` of the answer (`true`: the
    /// server holds no key for the id).
    pub(super) async fn register_peer(&mut self) -> bool {
        let msg = rv(|m| {
            m.set_register_peer(RegisterPeer {
                id: TARGET.to_owned(),
                ..Default::default()
            })
        });
        self.send(&msg).await;
        match self.recv().await.union {
            Some(rendezvous_message::Union::RegisterPeerResponse(r)) => r.request_pk,
            other => panic!("expected RegisterPeerResponse, got {other:?}"),
        }
    }

    async fn send(&mut self, msg: &RendezvousMessage) {
        self.0.send(&msg.write_to_bytes().unwrap()).await.unwrap();
    }

    async fn recv(&mut self) -> RendezvousMessage {
        let mut buf = vec![0; 64 * 1024];
        let n = timeout(STEP, self.0.recv(&mut buf)).await.unwrap().unwrap();
        RendezvousMessage::parse_from_bytes(&buf[..n]).unwrap()
    }
}

/// A controller: one TCP connection to the main port, kept open across requests.
struct Controller {
    conn: Framed<TcpStream, BytesCodec>,
    licence_key: String,
}

impl Controller {
    async fn connect(port: u16, licence_key: String) -> Self {
        let stream = TcpStream::connect((LOCALHOST, port)).await.unwrap();
        Self {
            conn: Framed::new(stream, BytesCodec::new()),
            licence_key,
        }
    }

    async fn punch_hole_request(&mut self) {
        let msg = rv(|m| {
            m.set_punch_hole_request(PunchHoleRequest {
                id: TARGET.to_owned(),
                licence_key: self.licence_key.clone(),
                nat_type: NatType::ASYMMETRIC.into(),
                ..Default::default()
            })
        });
        let bytes = msg.write_to_bytes().unwrap();
        self.conn.send(bytes.into()).await.unwrap();
    }
}

/// The PunchHole forwarded to `target` for a request on `controller`.
async fn punch(controller: &mut Controller, target: &mut Target) -> PunchHole {
    controller.punch_hole_request().await;
    match target.recv().await.union {
        Some(rendezvous_message::Union::PunchHole(ph)) => ph,
        other => panic!("expected PunchHole, got {other:?}"),
    }
}

/// Repeats [`punch`] on the same connection until `accept` holds (settings apply
/// asynchronously).
async fn wait_punch(
    controller: &mut Controller,
    target: &mut Target,
    accept: impl Fn(&PunchHole) -> bool,
) -> PunchHole {
    let deadline = Instant::now() + STEP;
    loop {
        let ph = punch(controller, target).await;
        if accept(&ph) {
            return ph;
        }
        assert!(Instant::now() < deadline, "never accepted: {ph:?}");
        sleep(POLL).await;
    }
}

/// Fails the first `failures` preloads, recording when each preload started.
struct FlakyStore {
    failures: usize,
    calls: AtomicUsize,
    attempts: Mutex<Vec<Instant>>,
}

impl FlakyStore {
    fn failing(failures: usize) -> Self {
        Self {
            failures,
            calls: AtomicUsize::new(0),
            attempts: Mutex::new(Vec::new()),
        }
    }

    fn attempts(&self) -> Vec<Instant> {
        self.attempts.lock().unwrap().clone()
    }
}

#[async_trait]
impl PeerStore for FlakyStore {
    async fn load_page(
        &self,
        after: Option<&str>,
        _: usize,
    ) -> Result<Vec<PeerRecord>, StoreError> {
        if after.is_none() {
            self.attempts.lock().unwrap().push(Instant::now());
        }
        if self.calls.fetch_add(1, Ordering::SeqCst) < self.failures {
            return Err(StoreError::backend("store down"));
        }
        Ok(Vec::new())
    }

    async fn get(&self, _: &str) -> Result<Option<PeerRecord>, StoreError> {
        Ok(None)
    }

    async fn upsert_pk(&self, _: &PeerUpsert) -> Result<(), StoreError> {
        Ok(())
    }
}
