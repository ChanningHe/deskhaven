//! A signal server on free loopback ports for one test.

use super::{PeerKeys, STEP, TcpClient, UdpClient, WsClient, addr, free_base_port, msg};
use deskhaven_signal::{
    MemoryPeerStore, PeerStore, SignalConfig, SignalError, SignalHandle, SignalServer,
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
use std::{net::SocketAddr, sync::Arc};
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

/// A fresh, empty, non-persistent store.
pub fn memory_store() -> Arc<dyn PeerStore> {
    Arc::new(MemoryPeerStore::new())
}

/// Everything a server is built from, kept so [`TestServer::restart`] can rebuild the same
/// server (same ports, same key, same `PeerStore` instance and so the same registrations).
#[derive(Clone)]
struct Setup {
    cfg: SignalConfig,
    store: Arc<dyn PeerStore>,
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
        for _ in 0..START_ATTEMPTS {
            let cfg = customize(config(free_base_port()));
            let setup = Setup {
                cfg,
                store: store.clone(),
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
        let store = memory_store();
        Self::start_setup(Setup { cfg, store }).await
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
        let server = SignalServer::new(cfg, setup.store.clone());
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
