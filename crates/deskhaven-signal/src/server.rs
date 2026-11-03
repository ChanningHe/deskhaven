//! Embedding API (deskhaven code, not vendored): configuration, lifecycle and runtime control
//! of the signal server. Replaces upstream's command line, environment variables, process
//! signal handling and loopback console.

use crate::common::{RESOLVE_TIMEOUT, resolve_servers};
use crate::control::ControlPlane;
use crate::rendezvous_server::{Data, Receiver, RendezvousServer, Sender};
use crate::store::{PeerStore, StoreError};
use crate::tasks::Tasks;
use hbb_common::{
    anyhow,
    futures::FutureExt as _,
    log,
    tokio::{
        self,
        sync::{mpsc, oneshot, watch},
    },
};
use ipnetwork::Ipv4Network;
use sodiumoxide::crypto::sign;
use std::{
    any::Any,
    net::IpAddr,
    panic::AssertUnwindSafe,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

/// Upstream default of `MAX_PUNCH_REQUESTS`.
pub const DEFAULT_PUNCH_REQ_MAX: usize = 100_000;
/// Default bound of loading the peer registry at start.
pub const DEFAULT_PEER_PRELOAD_TIMEOUT: Duration = Duration::from_secs(30);
/// Upstream default port (`hbb_common::config::RENDEZVOUS_PORT`).
const DEFAULT_PORT: u16 = 21116;
/// Upstream rejects main ports below 3; the NAT test port is `port - 1`.
const MIN_PORT: u16 = 3;
/// The websocket port is `port + 2`.
const MAX_PORT: u16 = u16::MAX - 2;

type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Everything the server needs; nothing is read from the environment or the working directory.
#[derive(Clone, Debug)]
pub struct SignalConfig {
    /// Address every listener binds to. `None`: all interfaces (IPv6 dual-stack, falling back
    /// to IPv4), like upstream without `--bind`.
    pub bind: Option<IpAddr>,
    /// Main TCP + UDP port (upstream `-p`). The server also listens on TCP `port - 1` (NAT
    /// test and online queries) and TCP `port + 2` (websocket); valid range 3..=65533.
    pub port: u16,
    /// UDP receive buffer size in bytes, 0 = OS default (upstream `--rmem`).
    pub rmem: usize,
    /// LAN range (upstream `--mask`): peers inside it are handed `local_ip` as relay.
    pub mask: Option<Ipv4Network>,
    /// Relay address for LAN peers when `mask` is set (upstream `LOCAL_IP`). `None` or empty:
    /// the detected local IP. A string because it stands in for a `relay_servers` entry.
    pub local_ip: Option<String>,
    /// Initial relay servers, `host[:port]` (upstream `-r`); empty entries are dropped. They
    /// are resolved off the server loop after the listeners are bound (each lookup bounded by
    /// 3 s); entries that do not resolve are logged, left out, and retried every 3 s until they
    /// do or the list is replaced with [`SignalHandle::set_relay_servers`].
    pub relay_servers: Vec<String>,
    /// Force relayed connections (upstream `ALWAYS_USE_RELAY`). Change at runtime with
    /// [`SignalHandle::set_always_use_relay`].
    pub always_use_relay: bool,
    /// Refuse KeyExchange replies that pick version 0 (clients from before Kx v1, one key for
    /// both directions) by closing the connection (M1-07). Clients that skip the exchange
    /// (not logged in) are not affected. Change at runtime with
    /// [`SignalHandle::set_require_kx_v1`].
    pub require_kx_v1: bool,
    /// Bound of the in-memory punch request log, 0 disables it (upstream `MAX_PUNCH_REQUESTS`).
    pub punch_req_max: usize,
    /// Bound of loading every registration from the [`PeerStore`] at start, before the
    /// listeners are bound; exceeding it (or a store error) fails [`SignalServer::run`] with
    /// [`SignalError::PeerPreload`].
    pub peer_preload_timeout: Duration,
    /// hbbs ed25519 key: signs peer public keys (`IdPk`), and clients must present its public
    /// key as licence key (upstream default `-k -`). Loading/generating the key file is the
    /// caller's job.
    pub secret_key: sign::SecretKey,
    /// Who a websocket client is when a reverse proxy sits in front of the websocket port
    /// (`X-Forwarded-For` and the like): the server never trusts those headers itself. The
    /// default, [`SocketPeerIp`], ignores them.
    pub ws_client_ip: Arc<dyn ClientIpResolver>,
}

/// The handshake headers of a websocket connection.
pub type WsHeaders = tungstenite::http::HeaderMap;

/// Decides a websocket client's IP from the socket peer and the handshake headers. The IP
/// feeds per-IP registration limits, the anti id-hijack IP check, LAN detection and relay
/// choice, so only headers from trusted proxies may change it.
pub trait ClientIpResolver: std::fmt::Debug + Send + Sync + 'static {
    /// The client's IP: `peer` itself unless `peer` is a trusted proxy.
    fn client_ip(&self, peer: IpAddr, headers: &WsHeaders) -> IpAddr;
}

/// Trusts no header: the client is the socket peer.
#[derive(Clone, Copy, Debug, Default)]
pub struct SocketPeerIp;

impl ClientIpResolver for SocketPeerIp {
    fn client_ip(&self, peer: IpAddr, _headers: &WsHeaders) -> IpAddr {
        peer
    }
}

impl SignalConfig {
    /// Upstream defaults around the given key.
    pub fn new(secret_key: sign::SecretKey) -> Self {
        Self {
            bind: None,
            port: DEFAULT_PORT,
            rmem: 0,
            mask: None,
            local_ip: None,
            relay_servers: Vec::new(),
            always_use_relay: false,
            require_kx_v1: false,
            punch_req_max: DEFAULT_PUNCH_REQ_MAX,
            peer_preload_timeout: DEFAULT_PEER_PRELOAD_TIMEOUT,
            secret_key,
            ws_client_ip: Arc::new(SocketPeerIp),
        }
    }

    /// The licence key clients must send: base64 of the public key.
    pub fn licence_key(&self) -> String {
        base64::encode(self.secret_key.public_key())
    }

    pub(crate) fn ports(&self) -> Result<Ports, SignalError> {
        let main = self.port;
        if !(MIN_PORT..=MAX_PORT).contains(&main) {
            return Err(SignalError::InvalidPort(main));
        }
        Ok(Ports {
            main,
            nat: main - 1,
            ws: main + 2,
        })
    }
}

/// The three ports derived from [`SignalConfig::port`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ports {
    pub(crate) main: u16,
    pub(crate) nat: u16,
    pub(crate) ws: u16,
}

/// Why the server could not start, stopped serving, or rejected a handle call. Every error
/// returned by [`SignalServer::run`] is terminal: the server is gone and nothing restarts it, so
/// the embedding process must supervise it (restart or exit).
#[derive(Debug, thiserror::Error)]
pub enum SignalError {
    /// [`SignalConfig::port`] leaves no room for the derived NAT test / websocket ports.
    #[error(
        "invalid signal port {0}: must be within {MIN_PORT}..={MAX_PORT} \
         (TCP port-1 serves NAT tests, TCP port+2 websocket)"
    )]
    InvalidPort(u16),
    /// A listener could not be bound, at startup or when re-created after an accept / receive
    /// failure (e.g. the port is taken).
    #[error("cannot listen on {proto} port {port}")]
    Bind {
        /// `"tcp"` or `"udp"`.
        proto: &'static str,
        /// The port that could not be bound.
        port: u16,
        /// The underlying bind error.
        #[source]
        source: BoxError,
    },
    /// The periodic UDP self-test got no answer from the server's own port for 12 s
    /// (upstream's guard against a wedged UDP socket; upstream exited the process).
    #[error("UDP self-test failed")]
    SelfTest(#[source] BoxError),
    /// Unexpected internal failure (e.g. reading a listener's local address).
    #[error("signal server failed")]
    Internal(#[source] BoxError),
    /// The peer registry could not be loaded from the [`PeerStore`] at start (store error or
    /// [`SignalConfig::peer_preload_timeout`] exceeded); nothing was bound.
    #[error("cannot load the peer registry")]
    PeerPreload(#[source] StoreError),
    /// [`SignalHandle::set_relay_servers`] got entries that do not resolve; nothing was applied.
    #[error("relay servers do not resolve: {}", rejected.join(", "))]
    RelayServersRejected {
        /// The offending entries, in input order.
        rejected: Vec<String>,
    },
    /// The server behind a [`SignalHandle`] is not running any more: its `run` future finished
    /// or was dropped, or its [`SignalServer`] was dropped without running.
    #[error("signal server is not running")]
    Stopped,
}

impl SignalError {
    /// An I/O error stays the source itself, so callers can match its kind (e.g. `AddrInUse`).
    pub(crate) fn bind(proto: &'static str, port: u16, err: anyhow::Error) -> Self {
        let source = match err.downcast::<std::io::Error>() {
            Ok(io) => io.into(),
            Err(err) => err.into(),
        };
        Self::Bind {
            proto,
            port,
            source,
        }
    }

    pub(crate) fn self_test(err: anyhow::Error) -> Self {
        Self::SelfTest(err.into())
    }

    pub(crate) fn internal(err: impl Into<BoxError>) -> Self {
        Self::Internal(err.into())
    }
}

/// Runtime control and state of a server; replaces upstream's loopback console commands `rs`
/// and `aur`.
///
/// Calls fail with [`SignalError::Stopped`] once the server's `run` future has finished or been
/// dropped. Updates travel to the server loop over an unbounded channel; that is fine because
/// they are rare, admin-driven events.
#[derive(Clone, Debug)]
pub struct SignalHandle {
    tx: Sender,
    switches: Switches,
    serving: watch::Receiver<bool>,
}

/// deskhaven (M2-19): a one-shot reply carried in the loop's `Data` (which is `Clone` +
/// `Debug`): the first `send` delivers, later ones and clones find it taken.
#[derive(Clone)]
pub(crate) struct ReplySlot<T>(Arc<std::sync::Mutex<Option<oneshot::Sender<T>>>>);

impl<T> ReplySlot<T> {
    fn new() -> (Self, oneshot::Receiver<T>) {
        let (tx, rx) = oneshot::channel();
        (Self(Arc::new(std::sync::Mutex::new(Some(tx)))), rx)
    }

    pub(crate) fn send(&self, value: T) {
        let taken = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(tx) = taken {
            // The asker may have given up; nothing to do then.
            let _ = tx.send(value);
        }
    }
}

impl<T> std::fmt::Debug for ReplySlot<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReplySlot")
    }
}

/// The runtime switches the handle sets and every connection reads (lock-free).
#[derive(Clone, Debug)]
pub(crate) struct Switches {
    pub(crate) always_use_relay: Arc<AtomicBool>,
    // deskhaven (M1-07): read when a KeyExchange reply arrives.
    pub(crate) require_kx_v1: Arc<AtomicBool>,
}

impl Switches {
    /// The initial values of `cfg`.
    pub(crate) fn new(cfg: &SignalConfig) -> Self {
        Self {
            always_use_relay: Arc::new(AtomicBool::new(cfg.always_use_relay)),
            require_kx_v1: Arc::new(AtomicBool::new(cfg.require_kx_v1)),
        }
    }
}

impl SignalHandle {
    pub(crate) fn new(tx: Sender, switches: Switches, serving: watch::Receiver<bool>) -> Self {
        Self {
            tx,
            switches,
            serving,
        }
    }

    /// Whether the server is serving: every listener was bound and `run` has not returned.
    /// False while the peer registry is still being loaded.
    pub fn is_serving(&self) -> bool {
        // A dropped `run` future never resets the flag, but it closes the channel.
        self.serving.has_changed().is_ok() && *self.serving.borrow()
    }

    /// Waits until the server's `run` has returned (every task it spawned finished, the
    /// registration writer included) or its future was dropped; at once if it never ran
    /// because the [`SignalServer`] was dropped.
    pub async fn stopped(&self) {
        let mut serving = self.serving.clone();
        // Never true: resolves when the last sender (dropped as `run` returns) is gone.
        let _ = serving.wait_for(|_| false).await;
    }

    /// Test-only: makes the running server fail as if it hit `fault`.
    #[cfg(any(test, feature = "testing"))]
    #[doc(hidden)]
    pub fn inject_fault(&self, fault: Fault) -> Result<(), SignalError> {
        self.tx
            .send(Data::Inject(fault))
            .map_err(|_| SignalError::Stopped)
    }

    /// Waits until the server is serving; [`SignalError::Stopped`] if its `run` ends first
    /// (or already ended, or never started because the [`SignalServer`] was dropped).
    pub async fn serving(&self) -> Result<(), SignalError> {
        let mut serving = self.serving.clone();
        serving
            .wait_for(|on| *on)
            .await
            .map(drop)
            .map_err(|_| SignalError::Stopped)
    }

    /// Replaces the relay list (`host[:port]`). Every entry is resolved here, on the caller's
    /// task, never on the server loop: concurrently, each bounded by a 3 s timeout; empty
    /// entries are dropped. If any entry does not resolve in time nothing is applied and
    /// [`SignalError::RelayServersRejected`] names them; otherwise the accepted list is
    /// returned and every relay choice from then on uses it, on new and already open
    /// connections alike (until the health check narrows it).
    ///
    /// Concurrent callers must be serialized by the caller (the control plane does): two
    /// overlapping calls are applied in the order their lookups finish, not the order they
    /// were made.
    pub async fn set_relay_servers(
        &self,
        servers: Vec<String>,
    ) -> Result<Vec<String>, SignalError> {
        self.ensure_running()?;
        let (accepted, rejected) = resolve_servers(&servers, RESOLVE_TIMEOUT).await;
        if !rejected.is_empty() {
            log::warn!("relay-servers rejected (do not resolve): {rejected:?}");
            return Err(SignalError::RelayServersRejected { rejected });
        }
        log::info!("relay-servers={accepted:?}");
        self.tx
            .send(Data::RelayServers0(accepted.clone()))
            .map_err(|_| SignalError::Stopped)?;
        Ok(accepted)
    }

    /// Takes effect immediately for every connection.
    pub fn set_always_use_relay(&self, on: bool) -> Result<(), SignalError> {
        self.ensure_running()?;
        self.switches.always_use_relay.store(on, Ordering::SeqCst);
        log::debug!("ALWAYS_USE_RELAY={}", if on { "Y" } else { "N" });
        Ok(())
    }

    /// See [`SignalConfig::require_kx_v1`]. Applies to every KeyExchange reply received from
    /// then on, on new and already open connections alike; connections secured before keep
    /// their cipher.
    pub fn set_require_kx_v1(&self, on: bool) -> Result<(), SignalError> {
        self.ensure_running()?;
        self.switches.require_kx_v1.store(on, Ordering::SeqCst);
        log::debug!("require_kx_v1={on}");
        Ok(())
    }

    /// Re-reads the registration of `id` from the [`PeerStore`](crate::PeerStore), for a
    /// registry change made behind the server's back: a deleted device is forgotten (its next
    /// RegisterPk registers it again, as a new device), a changed one takes the stored keys.
    /// Asynchronous: returns once queued; the store read runs in its own task.
    pub fn reload_peer(&self, id: &str) -> Result<(), SignalError> {
        self.tx
            .send(Data::ReloadPeer(id.to_owned()))
            .map_err(|_| SignalError::Stopped)
    }

    /// The uuid of `id`'s registration in the server's memory (`None`: not registered, or no
    /// uuid yet), whether or not the store has it yet: registrations are written behind.
    /// For changes made behind the server's back that must not contradict a live device.
    pub async fn live_uuid(&self, id: &str) -> Result<Option<Vec<u8>>, SignalError> {
        let (slot, rx) = ReplySlot::new();
        self.tx
            .send(Data::LiveUuid(id.to_owned(), slot))
            .map_err(|_| SignalError::Stopped)?;
        rx.await.map_err(|_| SignalError::Stopped)
    }

    fn ensure_running(&self) -> Result<(), SignalError> {
        if self.tx.is_closed() {
            return Err(SignalError::Stopped);
        }
        Ok(())
    }
}

/// Test-only faults for [`SignalHandle::inject_fault`] (feature `testing`).
#[cfg(any(test, feature = "testing"))]
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// The server loop returns an internal error, like a failed rebind.
    Error,
    /// The server loop panics.
    Panic,
}

/// A configured server that is not running yet.
pub struct SignalServer {
    cfg: SignalConfig,
    store: Arc<dyn PeerStore>,
    control: Arc<dyn ControlPlane>,
    handle: SignalHandle,
    rx: Receiver,
    serving: watch::Sender<bool>,
}

impl SignalServer {
    /// Prepares a server that keeps peer registrations in `store`; nothing is bound or
    /// resolved until [`SignalServer::run`].
    ///
    /// The store is what binds a registered id to its device uuid across restarts (the
    /// UUID_MISMATCH anti id-hijack check); with a non-persistent store such as
    /// [`MemoryPeerStore`](crate::MemoryPeerStore), any uuid can claim any id after a process
    /// restart.
    ///
    /// `control` makes the policy decisions ([`AllowAll`](crate::AllowAll): upstream's).
    pub fn new(
        cfg: SignalConfig,
        store: Arc<dyn PeerStore>,
        control: Arc<dyn ControlPlane>,
    ) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let switches = Switches::new(&cfg);
        let (serving, serving_rx) = watch::channel(false);
        Self {
            cfg,
            store,
            control,
            handle: SignalHandle::new(tx, switches, serving_rx),
            rx,
            serving,
        }
    }

    /// A handle for changing relay settings and watching the state while the server runs;
    /// take it before `run`.
    pub fn handle(&self) -> SignalHandle {
        self.handle.clone()
    }

    /// Resolves the initial relay list, binds, then serves on the caller's runtime until
    /// `shutdown` is cancelled (`Ok`) or a listener cannot be (re)created or the UDP self-test
    /// fails (`Err`). When it returns, every listener is closed, every task it spawned has
    /// finished, and every accepted registration has been handed to the store (bounded by a
    /// few seconds; anything left is logged as lost).
    ///
    /// Errors are terminal: the server does not restart itself, and the embedding process
    /// must supervise it (restart it or exit).
    pub async fn run(self, shutdown: CancellationToken) -> Result<(), SignalError> {
        let ports = self.cfg.ports()?;
        // deskhaven (M1-07 review): libsodium is initialised before any connection generates
        // KeyExchange keys (thread-safe, idempotent; its RNG is otherwise set up lazily).
        sodiumoxide::init()
            .map_err(|()| SignalError::internal("libsodium failed to initialise"))?;
        let stop = shutdown.child_token();
        // Also stops the tasks if this future is dropped instead of awaited to the end.
        let _stop_on_drop = stop.clone().drop_guard();
        let tasks = Tasks::new(stop);
        let serving = self.serving.clone();
        // deskhaven (M1-06): a panic of the server loop must not skip the drain below (a
        // registration writer still flushing would race a restarted instance or the
        // database close); it becomes an internal error, which the embedder restarts.
        let res = AssertUnwindSafe(self.serve_until(ports, tasks.clone(), &shutdown))
            .catch_unwind()
            .await
            .unwrap_or_else(|panic| Err(SignalError::internal(panic_message(&*panic))));
        // Not serving any more from here on (readiness drops before the drain).
        serving.send_replace(false);
        tasks.shutdown().await;
        res
    }

    async fn serve_until(
        self,
        ports: Ports,
        tasks: Tasks,
        shutdown: &CancellationToken,
    ) -> Result<(), SignalError> {
        let Self {
            cfg,
            store,
            control,
            handle,
            rx,
            serving,
        } = self;
        let serve = RendezvousServer::serve(
            cfg,
            ports,
            store,
            control,
            handle.tx,
            rx,
            handle.switches,
            tasks,
            &serving,
        );
        tokio::select! {
            biased;
            res = serve => res,
            () = shutdown.cancelled() => {
                log::info!("signal server stopped");
                Ok(())
            }
        }
    }
}

fn panic_message(panic: &(dyn Any + Send)) -> String {
    let text = panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string payload");
    format!("signal server panicked: {text}")
}

/// [`SignalServer::run`] for callers that never change relay settings at runtime.
pub async fn run(
    cfg: SignalConfig,
    store: Arc<dyn PeerStore>,
    control: Arc<dyn ControlPlane>,
    shutdown: CancellationToken,
) -> Result<(), SignalError> {
    SignalServer::new(cfg, store, control).run(shutdown).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(port: u16) -> SignalConfig {
        SignalConfig {
            port,
            ..SignalConfig::new(sign::gen_keypair().1)
        }
    }

    #[test]
    fn defaults_match_upstream() {
        let c = cfg(DEFAULT_PORT);
        assert_eq!(
            i32::from(c.port),
            hbb_common::config::RENDEZVOUS_PORT,
            "default port"
        );
        assert_eq!(c.bind, None);
        assert_eq!(c.punch_req_max, DEFAULT_PUNCH_REQ_MAX);
        assert!(!c.always_use_relay && c.relay_servers.is_empty() && c.mask.is_none());
        assert!(!c.require_kx_v1, "clients from before Kx v1 are served");
    }

    #[test]
    fn ports_derive_nat_and_ws_and_reject_out_of_range() {
        let p = cfg(DEFAULT_PORT).ports().unwrap();
        assert_eq!((p.nat, p.main, p.ws), (21115, 21116, 21118));
        assert!(cfg(MIN_PORT).ports().is_ok() && cfg(MAX_PORT).ports().is_ok());
        for port in [0, 1, MIN_PORT - 1, MAX_PORT + 1, u16::MAX] {
            assert!(matches!(cfg(port).ports(), Err(SignalError::InvalidPort(p)) if p == port));
        }
    }

    #[test]
    fn licence_key_is_base64_public_key() {
        let (pk, sk) = sign::gen_keypair();
        let c = SignalConfig::new(sk);
        assert_eq!(base64::decode(c.licence_key()).unwrap(), pk.0.to_vec());
    }

    #[test]
    fn debug_output_redacts_secret_key() {
        let c = cfg(DEFAULT_PORT);
        let secret = base64::encode(&c.secret_key.0[..]);
        let debug = format!("{c:?}");
        assert!(
            !debug.contains(&secret) && debug.contains("****"),
            "{debug}"
        );
    }

    // deskhaven (M1-06): a panicking server loop still drains before `run` returns (the
    // registration writer finishes its in-flight write), readiness drops before that drain,
    // `stopped` resolves only after it, and the panic becomes an internal error.
    #[hbb_common::tokio::test]
    async fn panic_in_the_server_loop_drains_before_run_returns() {
        use crate::store::{PeerRecord, PeerUpsert};
        use hbb_common::protobuf::Message as _;
        use hbb_common::rendezvous_proto::{RegisterPk, RendezvousMessage};
        use std::sync::atomic::AtomicUsize;

        /// Every write takes `WRITE`; counts writes started and finished.
        #[derive(Default)]
        struct SlowStore {
            started: AtomicUsize,
            finished: AtomicUsize,
        }
        const WRITE: Duration = Duration::from_millis(1500);
        #[async_trait::async_trait]
        impl PeerStore for SlowStore {
            async fn load_page(
                &self,
                _: Option<&str>,
                _: usize,
            ) -> Result<Vec<PeerRecord>, StoreError> {
                Ok(Vec::new())
            }
            async fn get(&self, _: &str) -> Result<Option<PeerRecord>, StoreError> {
                Ok(None)
            }
            async fn upsert_pk(&self, _: &PeerUpsert) -> Result<(), StoreError> {
                self.started.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(WRITE).await;
                self.finished.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }
        async fn until(cond: impl Fn() -> bool) {
            for _ in 0..250 {
                if cond() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            panic!("condition never held");
        }

        // Arrange: a serving server with a registration write in flight.
        let localhost = IpAddr::from([127, 0, 0, 1]);
        let store = Arc::new(SlowStore::default());
        let (handle, task, port) = loop {
            let port = std::net::TcpListener::bind((localhost, 0))
                .unwrap()
                .local_addr()
                .unwrap()
                .port();
            let cfg = SignalConfig {
                bind: Some(localhost),
                ..cfg(port)
            };
            let server = SignalServer::new(cfg, store.clone(), Arc::new(crate::AllowAll::new()));
            let handle = server.handle();
            let task = tokio::spawn(server.run(CancellationToken::new()));
            if handle.serving().await.is_ok() {
                break (handle, task, port);
            }
            assert!(matches!(task.await.unwrap(), Err(SignalError::Bind { .. })));
        };
        let mut msg = RendezvousMessage::new();
        msg.set_register_pk(RegisterPk {
            id: "123456789".to_owned(),
            uuid: b"uuid".to_vec().into(),
            pk: vec![1; 32].into(),
            ..Default::default()
        });
        let udp = tokio::net::UdpSocket::bind((localhost, 0)).await.unwrap();
        udp.send_to(&msg.write_to_bytes().unwrap(), (localhost, port))
            .await
            .unwrap();
        until(|| store.started.load(Ordering::SeqCst) == 1).await;

        // Act
        handle.inject_fault(Fault::Panic).unwrap();

        // Assert
        until(|| !handle.is_serving()).await;
        assert_eq!(
            store.finished.load(Ordering::SeqCst),
            0,
            "not ready before the drain"
        );
        let stopped = tokio::spawn({
            let handle = handle.clone();
            async move { handle.stopped().await }
        });
        let res = tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(store.finished.load(Ordering::SeqCst), 1, "writer drained");
        match res {
            Err(err @ SignalError::Internal(_)) => {
                assert!(format!("{:?}", err).contains("injected panic"), "{err:?}")
            }
            other => panic!("expected an internal error, got {other:?}"),
        }
        tokio::time::timeout(Duration::from_secs(1), stopped)
            .await
            .unwrap()
            .unwrap();
    }
}
