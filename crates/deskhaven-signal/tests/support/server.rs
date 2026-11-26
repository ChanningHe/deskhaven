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

pub fn config(port: u16) -> SignalConfig {
    SignalConfig {
        bind: Some(super::LOCALHOST),
        port,
        ..SignalConfig::new(sign::gen_keypair().1)
    }
}

async fn all_tcp_ports_accept(port: u16, ws: SocketAddr) -> bool {
    for a in [addr(port - 1), addr(port), ws] {
        if TcpStream::connect(a).await.is_err() {
            return false;
        }
    }
    true
}

fn ws_listen_of(cfg: &SignalConfig) -> SocketAddr {
    cfg.ws_listen.unwrap_or_else(|| addr(cfg.port + 2))
}

/// `true` once every TCP port accepts, `false` if `run` ended first (a lost port race).
pub async fn wait_until_ready(port: u16, task: &RunTask) -> bool {
    wait_until_ready_at(port, addr(port + 2), task).await
}

async fn wait_until_ready_at(port: u16, ws: SocketAddr, task: &RunTask) -> bool {
    let deadline = Instant::now() + STEP;
    while Instant::now() < deadline {
        if task.is_finished() {
            return false;
        }
        if all_tcp_ports_accept(port, ws).await {
            return true;
        }
        sleep(READY_POLL).await;
    }
    panic!("signal server on port {port} did not become ready");
}

/// Trusts the last `X-Forwarded-For` entry from `proxies`, like a single trusted proxy hop.
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

pub fn trusting(proxies: &[IpAddr]) -> impl Fn(SignalConfig) -> SignalConfig {
    let proxies = proxies.to_vec();
    move |cfg| SignalConfig {
        ws_client_ip: Arc::new(TrustXff(proxies.clone())),
        ..cfg
    }
}

pub fn memory_store() -> Arc<dyn PeerStore> {
    Arc::new(MemoryPeerStore::new())
}

pub fn allow_all() -> Arc<dyn ControlPlane> {
    Arc::new(AllowAll::new())
}

/// Kept so [`TestServer::restart`] rebuilds the same server: ports, key, store and control plane.
#[derive(Clone)]
struct Setup {
    cfg: SignalConfig,
    store: Arc<dyn PeerStore>,
    control: Arc<dyn ControlPlane>,
}

pub struct TestServer {
    pub port: u16,
    pub handle: SignalHandle,
    pub licence_key: String,
    pub public_key: sign::PublicKey,
    setup: Setup,
    shutdown: CancellationToken,
    task: RunTask,
    _stop_on_drop: DropGuard,
}

impl TestServer {
    pub async fn start() -> Self {
        Self::start_with(|cfg| cfg).await
    }

    pub async fn start_with(customize: impl Fn(SignalConfig) -> SignalConfig) -> Self {
        Self::start_with_store(customize, memory_store()).await
    }

    pub async fn start_with_store(
        customize: impl Fn(SignalConfig) -> SignalConfig,
        store: Arc<dyn PeerStore>,
    ) -> Self {
        Self::start_with_parts(customize, store, allow_all()).await
    }

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

    pub async fn start_with_relays(relays: &[&str]) -> Self {
        let relays: Vec<String> = relays.iter().map(|r| (*r).to_owned()).collect();
        Self::start_with(|cfg| SignalConfig {
            relay_servers: relays.clone(),
            ..cfg
        })
        .await
    }

    pub async fn start_on(cfg: SignalConfig) -> Self {
        let setup = Setup {
            cfg,
            store: memory_store(),
            control: allow_all(),
        };
        Self::start_setup(setup).await
    }

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
        if !wait_until_ready_at(port, ws_listen_of(&setup.cfg), &task).await {
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

    pub async fn stop(self) {
        self.shutdown.cancel();
        let res = timeout(STEP, self.task)
            .await
            .expect("run returns promptly");
        assert!(matches!(res, Ok(Ok(()))), "run returns Ok: {res:?}");
    }

    pub fn main_addr(&self) -> SocketAddr {
        addr(self.port)
    }

    pub fn nat_addr(&self) -> SocketAddr {
        addr(self.port - 1)
    }

    pub fn ws_addr(&self) -> SocketAddr {
        ws_listen_of(&self.setup.cfg)
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

    pub async fn raw_connect(addr: SocketAddr) -> TcpStream {
        io("raw tcp connect", TcpStream::connect(addr))
            .await
            .unwrap()
    }

    /// Returns once every connection opened before the call is served, so a paused clock can
    /// move on safely: a witness connection per port gets a full round trip, and a listener
    /// serves in connection order. Event driven, no sleeps.
    pub async fn settle(&self) {
        read_offer(&mut self.tcp().await).await;
        self.nat().await.test_nat().await;
        self.ws().await;
    }

    pub fn punch_req(&self, target: &str) -> PunchHoleRequest {
        msg::punch_hole_request(target, &self.licence_key)
    }

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
