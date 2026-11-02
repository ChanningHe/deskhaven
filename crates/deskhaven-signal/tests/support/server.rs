//! A signal server on free loopback ports for one test.

use super::{
    PeerKeys, Rendezvous, STEP, TcpClient, UdpClient, WsClient, addr, free_base_port, io, msg,
    read_offer,
};
use deskhaven_signal::{
    AllowAll, ClientIpResolver, ControlPlane, MemoryPeerStore, PeerStore, SignalConfig,
    SignalError, SignalHandle, SignalServer, WsHeaders,
};
use hbb_common::{
    rendezvous_proto::{PunchHoleRequest, register_pk_response},
    tokio::{
        self,
        net::TcpStream,
        task::JoinHandle,
        time::{Duration, Instant, sleep, timeout},
    },
};
use sodiumoxide::crypto::sign;
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
};
use tokio_util::sync::{CancellationToken, DropGuard};

const READY_POLL: Duration = Duration::from_millis(20);
const START_ATTEMPTS: usize = 10;

type RunTask = JoinHandle<Result<(), SignalError>>;

/// Loopback-only config with a fresh key.
pub fn config(port: u16) -> SignalConfig {
    SignalConfig {
        bind: Some(super::LOCALHOST),
        port,
        ..SignalConfig::new(sign::gen_keypair().1)
    }
}

async fn all_tcp_ports_accept(port: u16) -> bool {
    for p in [port - 1, port, port + 2] {
        if TcpStream::connect(addr(p)).await.is_err() {
            return false;
        }
    }
    true
}

/// `true` once every TCP port accepts, `false` if `run` ended first (a lost port race).
pub async fn wait_until_ready(port: u16, task: &RunTask) -> bool {
    let deadline = Instant::now() + STEP;
    while Instant::now() < deadline {
        if task.is_finished() {
            return false;
        }
        if all_tcp_ports_accept(port).await {
            return true;
        }
        sleep(READY_POLL).await;
    }
    panic!("signal server on port {port} did not become ready");
}

/// Trusts the last `X-Forwarded-For` entry of websocket connections from `proxies` (what
/// deskhaven's `security.trusted_proxies` does for a single proxy hop); other peers' headers
/// are ignored.
#[derive(Debug)]
pub struct TrustXff(pub Vec<IpAddr>);

impl ClientIpResolver for TrustXff {
    fn client_ip(&self, peer: IpAddr, headers: &WsHeaders) -> IpAddr {
        if !self.0.contains(&peer.to_canonical()) {
            return peer;
        }
        headers
            .get_all("x-forwarded-for")
            .iter()
            .next_back()
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit(',').next())
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(peer)
    }
}

/// A config whose websocket port trusts `X-Forwarded-For` from `proxies` ([`TrustXff`]).
pub fn trusting(proxies: &[IpAddr]) -> impl Fn(SignalConfig) -> SignalConfig {
    let proxies = proxies.to_vec();
    move |cfg| SignalConfig {
        ws_client_ip: Arc::new(TrustXff(proxies.clone())),
        ..cfg
    }
}

/// A fresh, empty, non-persistent store.
pub fn memory_store() -> Arc<dyn PeerStore> {
    Arc::new(MemoryPeerStore::new())
}

/// Upstream's policy (a fresh relay rotation cursor).
pub fn allow_all() -> Arc<dyn ControlPlane> {
    Arc::new(AllowAll::new())
}

/// Everything a server is built from, kept so [`TestServer::restart`] can rebuild the same
/// server (same ports, same key, same `PeerStore` and `ControlPlane` instances and so the
/// same registrations).
#[derive(Clone)]
struct Setup {
    cfg: SignalConfig,
    store: Arc<dyn PeerStore>,
    control: Arc<dyn ControlPlane>,
}

pub struct TestServer {
    pub port: u16,
    pub handle: SignalHandle,
    /// What clients must send as `licence_key`.
    pub licence_key: String,
    /// Verifies the `IdPk`s the server signs.
    pub public_key: sign::PublicKey,
    setup: Setup,
    shutdown: CancellationToken,
    task: RunTask,
    /// Stops the server if a test panics before [`TestServer::stop`].
    _stop_on_drop: DropGuard,
}

impl TestServer {
    pub async fn start() -> Self {
        Self::start_with(|cfg| cfg).await
    }

    /// Starts with `customize(config(port))` on free ports and a fresh [`memory_store`],
    /// retrying on a lost port race.
    pub async fn start_with(customize: impl Fn(SignalConfig) -> SignalConfig) -> Self {
        Self::start_with_store(customize, memory_store()).await
    }

    /// [`TestServer::start_with`] on the given store.
    pub async fn start_with_store(
        customize: impl Fn(SignalConfig) -> SignalConfig,
        store: Arc<dyn PeerStore>,
    ) -> Self {
        Self::start_with_parts(customize, store, allow_all()).await
    }

    /// [`TestServer::start_with`] with the given control plane.
    pub async fn start_with_control(
        customize: impl Fn(SignalConfig) -> SignalConfig,
        control: Arc<dyn ControlPlane>,
    ) -> Self {
        Self::start_with_parts(customize, memory_store(), control).await
    }

    async fn start_with_parts(
        customize: impl Fn(SignalConfig) -> SignalConfig,
        store: Arc<dyn PeerStore>,
        control: Arc<dyn ControlPlane>,
    ) -> Self {
        for _ in 0..START_ATTEMPTS {
            let cfg = customize(config(free_base_port()));
            let setup = Setup {
                cfg,
                store: store.clone(),
                control: control.clone(),
            };
            if let Some(server) = Self::try_start(setup).await {
                return server;
            }
        }
        panic!("could not start the signal server");
    }

    /// Starts with the given initial relay list (`SignalConfig::relay_servers`).
    pub async fn start_with_relays(relays: &[&str]) -> Self {
        let relays: Vec<String> = relays.iter().map(|r| (*r).to_owned()).collect();
        Self::start_with(|cfg| SignalConfig {
            relay_servers: relays.clone(),
            ..cfg
        })
        .await
    }

    /// Starts on exactly `cfg` (its port and key) with a fresh [`memory_store`], retrying
    /// briefly while the ports are busy.
    pub async fn start_on(cfg: SignalConfig) -> Self {
        let setup = Setup {
            cfg,
            store: memory_store(),
            control: allow_all(),
        };
        Self::start_setup(setup).await
    }

    /// Stops the server and starts a new one on the same ports with the same key and store.
    pub async fn restart(self) -> Self {
        let setup = self.setup.clone();
        self.stop().await;
        Self::start_setup(setup).await
    }

    async fn start_setup(setup: Setup) -> Self {
        for _ in 0..START_ATTEMPTS {
            if let Some(server) = Self::try_start(setup.clone()).await {
                return server;
            }
            sleep(READY_POLL).await;
        }
        panic!(
            "could not start the signal server on port {}",
            setup.cfg.port
        );
    }

    async fn try_start(setup: Setup) -> Option<Self> {
        let cfg = setup.cfg.clone();
        let (port, licence_key) = (cfg.port, cfg.licence_key());
        let public_key = cfg.secret_key.public_key();
        let shutdown = CancellationToken::new();
        let server = SignalServer::new(cfg, setup.store.clone(), setup.control.clone());

        let handle = server.handle();
        let task = tokio::spawn(server.run(shutdown.clone()));
        if !wait_until_ready(port, &task).await {
            let res = task.await.unwrap();
            assert!(matches!(res, Err(SignalError::Bind { .. })), "{res:?}");
            return None;
        }
        let _stop_on_drop = shutdown.clone().drop_guard();
        Some(Self {
            port,
            handle,
            licence_key,
            public_key,
            setup,
            shutdown,
            task,
            _stop_on_drop,
        })
    }

    /// Cancels and checks that `run` returns `Ok` promptly.
    pub async fn stop(self) {
        self.shutdown.cancel();
        let res = timeout(STEP, self.task)
            .await
            .expect("run returns promptly");
        assert!(matches!(res, Ok(Ok(()))), "run returns Ok: {res:?}");
    }

    /// TCP + UDP main port.
    pub fn main_addr(&self) -> SocketAddr {
        addr(self.port)
    }

    /// TCP NAT test / online query port.
    pub fn nat_addr(&self) -> SocketAddr {
        addr(self.port - 1)
    }

    pub fn ws_addr(&self) -> SocketAddr {
        addr(self.port + 2)
    }

    pub async fn udp(&self) -> UdpClient {
        UdpClient::bind(self.main_addr()).await
    }

    pub async fn tcp(&self) -> TcpClient {
        TcpClient::connect(self.main_addr()).await
    }

    pub async fn nat(&self) -> TcpClient {
        TcpClient::connect(self.nat_addr()).await
    }

    pub async fn ws(&self) -> WsClient {
        WsClient::connect(self.ws_addr()).await
    }

    /// A bare TCP connection to `addr` that sends nothing (for timeout tests).
    pub async fn raw_connect(addr: SocketAddr) -> TcpStream {
        io("raw tcp connect", TcpStream::connect(addr))
            .await
            .unwrap()
    }

    /// Returns once the server has accepted and started serving every connection opened before
    /// the call, on all three ports, so a paused clock can safely move on: the server's per-
    /// connection timers exist and nothing of ours is still in flight. Each port gets a witness
    /// connection with a full round trip (main: the KeyExchange offer, NAT: a TestNatRequest,
    /// websocket: the handshake); a listener accepts and spawns in connection order, so the
    /// witness being served means the earlier connections already are. Event driven: no
    /// sleeps.
    pub async fn settle(&self) {
        read_offer(&mut self.tcp().await).await;
        self.nat().await.test_nat().await;
        self.ws().await;
    }

    /// A PunchHoleRequest for `target` with this server's licence key.
    pub fn punch_req(&self, target: &str) -> PunchHoleRequest {
        msg::punch_hole_request(target, &self.licence_key)
    }

    /// A controlled peer that registered its key over UDP (RegisterPk OK) and is online.
    pub async fn online_peer(&self, id: &str) -> (UdpClient, PeerKeys) {
        let mut peer = self.udp().await;
        let keys = PeerKeys::random();
        assert_eq!(
            peer.register_pk(id, &keys).await,
            register_pk_response::Result::OK
        );
        (peer, keys)
    }
}
