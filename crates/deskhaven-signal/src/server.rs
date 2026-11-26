use crate::accept_gate::AcceptLimits;
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
    net::{IpAddr, SocketAddr},
    panic::AssertUnwindSafe,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

pub const DEFAULT_PUNCH_REQ_MAX: usize = 100_000;
pub const DEFAULT_PEER_PRELOAD_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_PORT: u16 = 21116;
/// Upstream rejects main ports below 3; the NAT test port is `port - 1`.
const MIN_PORT: u16 = 3;
/// The websocket port is `port + 2`.
const MAX_PORT: u16 = u16::MAX - 2;

type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

#[derive(Clone, Debug)]
pub struct SignalConfig {
    /// `None`: all interfaces (IPv6 dual-stack, falling back to IPv4).
    pub bind: Option<IpAddr>,
    /// Main TCP + UDP port; the server also listens on TCP `port - 1` (NAT test) and
    /// `port + 2` (websocket).
    pub port: u16,
    /// UDP receive buffer size in bytes, 0 = OS default.
    pub rmem: usize,
    /// LAN range: peers inside it are handed `local_ip` as relay.
    pub mask: Option<Ipv4Network>,
    /// Relay for LAN peers when `mask` is set; `None` or empty: the detected local IP.
    pub local_ip: Option<String>,
    /// `host[:port]`; entries that do not resolve are left out and retried every 3 s.
    pub relay_servers: Vec<String>,
    pub always_use_relay: bool,
    /// Refuse KeyExchange replies that pick version 0 (one key for both directions); clients
    /// that skip the exchange are not affected.
    pub require_kx_v1: bool,
    /// Bound of the in-memory punch request log, 0 disables it.
    pub punch_req_max: usize,
    pub peer_preload_timeout: Duration,
    /// Signs peer public keys; clients present its public key as licence key.
    pub secret_key: sign::SecretKey,
    /// The server never trusts forwarding headers itself; this decides who a websocket client is.
    pub ws_client_ip: Arc<dyn ClientIpResolver>,
    /// Checked at accept, before the KeyExchange offer is computed.
    pub accept_limits: AcceptLimits,
    /// `None`: `bind` and `port + 2`.
    pub ws_listen: Option<SocketAddr>,
}

pub type WsHeaders = tungstenite::http::HeaderMap;

/// Only trusted proxies may change the IP: it feeds per-IP limits, the anti id-hijack check,
/// LAN detection and relay choice.
pub trait ClientIpResolver: std::fmt::Debug + Send + Sync + 'static {
    fn client_ip(&self, peer: IpAddr, headers: &WsHeaders) -> IpAddr;

    /// Trusted proxies carry many clients, so the accept gate exempts them from the per-source
    /// pending limits.
    fn is_trusted_proxy(&self, _peer: IpAddr) -> bool {
        false
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SocketPeerIp;

impl ClientIpResolver for SocketPeerIp {
    fn client_ip(&self, peer: IpAddr, _headers: &WsHeaders) -> IpAddr {
        peer
    }
}

impl SignalConfig {
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
            accept_limits: AcceptLimits::default(),
            ws_listen: None,
        }
    }

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
            ws: self.ws_listen.map_or(main + 2, |a| a.port()),
        })
    }

    pub(crate) fn ws_bind(&self) -> Option<IpAddr> {
        self.ws_listen.map(|a| a.ip()).or(self.bind)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Ports {
    pub(crate) main: u16,
    pub(crate) nat: u16,
    pub(crate) ws: u16,
}

/// Every error returned by [`SignalServer::run`] is terminal: the embedder must restart or exit.
#[derive(Debug, thiserror::Error)]
pub enum SignalError {
    #[error(
        "invalid signal port {0}: must be within {MIN_PORT}..={MAX_PORT} \
         (TCP port-1 serves NAT tests, TCP port+2 websocket)"
    )]
    InvalidPort(u16),
    #[error("cannot listen on {proto} port {port}")]
    Bind {
        proto: &'static str,
        port: u16,
        #[source]
        source: BoxError,
    },
    /// No answer from the server's own UDP port for 12 s (a wedged UDP socket).
    #[error("UDP self-test failed")]
    SelfTest(#[source] BoxError),
    #[error("signal server failed")]
    Internal(#[source] BoxError),
    #[error("cannot load the peer registry")]
    PeerPreload(#[source] StoreError),
    #[error("relay servers do not resolve: {}", rejected.join(", "))]
    RelayServersRejected {
        /// The offending entries, in input order.
        rejected: Vec<String>,
    },
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

/// Updates travel over an unbounded channel: they are rare, admin-driven events.
#[derive(Clone, Debug)]
pub struct SignalHandle {
    tx: Sender,
    switches: Switches,
    serving: watch::Receiver<bool>,
}

/// A one-shot reply carried in the loop's `Clone` data: the first `send` delivers, later ones
/// and clones find it taken.
pub(crate) struct ReplySlot<T>(Arc<std::sync::Mutex<Option<oneshot::Sender<T>>>>);

// Not derived: clones share the slot, so `T` need not be `Clone` (a `WebRoute` is not).
impl<T> Clone for ReplySlot<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

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

#[derive(Clone, Debug)]
pub(crate) struct Switches {
    pub(crate) always_use_relay: Arc<AtomicBool>,
    pub(crate) require_kx_v1: Arc<AtomicBool>,
}

impl Switches {
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

    pub fn is_serving(&self) -> bool {
        // A dropped `run` future never resets the flag, but it closes the channel.
        self.serving.has_changed().is_ok() && *self.serving.borrow()
    }

    pub async fn stopped(&self) {
        let mut serving = self.serving.clone();
        // Never true: resolves when the last sender (dropped as `run` returns) is gone.
        let _ = serving.wait_for(|_| false).await;
    }

    #[cfg(any(test, feature = "testing"))]
    #[doc(hidden)]
    pub fn inject_fault(&self, fault: Fault) -> Result<(), SignalError> {
        self.tx
            .send(Data::Inject(fault))
            .map_err(|_| SignalError::Stopped)
    }

    pub async fn serving(&self) -> Result<(), SignalError> {
        let mut serving = self.serving.clone();
        serving
            .wait_for(|on| *on)
            .await
            .map(drop)
            .map_err(|_| SignalError::Stopped)
    }

    /// Resolves every entry on the caller's task, never on the server loop, and applies nothing
    /// if any entry does not resolve. Concurrent callers must serialize: overlapping calls are
    /// applied in the order their lookups finish.
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

    pub fn set_always_use_relay(&self, on: bool) -> Result<(), SignalError> {
        self.ensure_running()?;
        self.switches.always_use_relay.store(on, Ordering::SeqCst);
        log::debug!("ALWAYS_USE_RELAY={}", if on { "Y" } else { "N" });
        Ok(())
    }

    pub fn set_require_kx_v1(&self, on: bool) -> Result<(), SignalError> {
        self.ensure_running()?;
        self.switches.require_kx_v1.store(on, Ordering::SeqCst);
        log::debug!("require_kx_v1={on}");
        Ok(())
    }

    /// For registry changes made behind the server's back; returns once queued.
    pub fn reload_peer(&self, id: &str) -> Result<(), SignalError> {
        self.tx
            .send(Data::ReloadPeer(id.to_owned()))
            .map_err(|_| SignalError::Stopped)
    }

    /// May be ahead of the store: registrations are written behind.
    pub async fn live_uuid(&self, id: &str) -> Result<Option<Vec<u8>>, SignalError> {
        let (slot, rx) = ReplySlot::new();
        self.tx
            .send(Data::LiveUuid(id.to_owned(), slot))
            .map_err(|_| SignalError::Stopped)?;
        rx.await.map_err(|_| SignalError::Stopped)
    }

    /// No [`ControlPlane`] hook is consulted: authorizing the browser is the caller's job.
    pub async fn web_punch(
        &self,
        req: crate::WebPunch,
    ) -> Result<crate::WebRoute, crate::WebPunchError> {
        let (slot, rx) = ReplySlot::new();
        self.tx
            .send(Data::WebPunch(req, slot))
            .map_err(|_| crate::WebPunchError::Stopped)?;
        rx.await.map_err(|_| crate::WebPunchError::Stopped)?
    }

    fn ensure_running(&self) -> Result<(), SignalError> {
        if self.tx.is_closed() {
            return Err(SignalError::Stopped);
        }
        Ok(())
    }
}

#[cfg(any(test, feature = "testing"))]
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    Error,
    Panic,
}

pub struct SignalServer {
    cfg: SignalConfig,
    store: Arc<dyn PeerStore>,
    control: Arc<dyn ControlPlane>,
    handle: SignalHandle,
    rx: Receiver,
    serving: watch::Sender<bool>,
}

impl SignalServer {
    /// The store binds ids to device uuids across restarts (the anti id-hijack check); with a
    /// non-persistent store any uuid can claim any id after a restart.
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

    pub fn handle(&self) -> SignalHandle {
        self.handle.clone()
    }

    pub async fn run(self, shutdown: CancellationToken) -> Result<(), SignalError> {
        let ports = self.cfg.ports()?;
        // libsodium must be initialised before any connection generates KeyExchange keys.
        sodiumoxide::init()
            .map_err(|()| SignalError::internal("libsodium failed to initialise"))?;
        let stop = shutdown.child_token();
        // Also stops the tasks if this future is dropped instead of awaited to the end.
        let _stop_on_drop = stop.clone().drop_guard();
        let tasks = Tasks::new(stop);
        let serving = self.serving.clone();
        // A panicking loop must still drain: a registration writer still flushing would race a
        // restarted instance or the database close.
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

    #[hbb_common::tokio::test]
    async fn panic_in_the_server_loop_drains_before_run_returns() {
        use crate::store::{PeerRecord, PeerUpsert};
        use hbb_common::protobuf::Message as _;
        use hbb_common::rendezvous_proto::{RegisterPk, RendezvousMessage};
        use std::sync::atomic::AtomicUsize;

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

        handle.inject_fault(Fault::Panic).unwrap();

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
