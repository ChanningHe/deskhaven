//! Embedding API (deskhaven code, not vendored): configuration, lifecycle and runtime control
//! of the signal server. Replaces upstream's command line, environment variables, process
//! signal handling and loopback console.

use crate::common::{RESOLVE_TIMEOUT, resolve_servers};
use crate::rendezvous_server::{Data, Receiver, RendezvousServer, Sender};
use crate::store::{PeerStore, StoreError};
use crate::tasks::Tasks;
use hbb_common::{
    anyhow, log,
    tokio::{self, sync::mpsc},
};
use ipnetwork::Ipv4Network;
use sodiumoxide::crypto::sign;
use std::{
    net::IpAddr,
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
            punch_req_max: DEFAULT_PUNCH_REQ_MAX,
            peer_preload_timeout: DEFAULT_PEER_PRELOAD_TIMEOUT,
            secret_key,
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
    pub(crate) fn bind(proto: &'static str, port: u16, err: anyhow::Error) -> Self {
        Self::Bind {
            proto,
            port,
            source: err.into(),
        }
    }

    pub(crate) fn self_test(err: anyhow::Error) -> Self {
        Self::SelfTest(err.into())
    }

    pub(crate) fn internal(err: impl Into<BoxError>) -> Self {
        Self::Internal(err.into())
    }
}

/// Runtime control of a server; replaces upstream's loopback console commands `rs` and `aur`.
///
/// Calls fail with [`SignalError::Stopped`] once the server's `run` future has finished or been
/// dropped. Updates travel to the server loop over an unbounded channel; that is fine because
/// they are rare, admin-driven events.
#[derive(Clone, Debug)]
pub struct SignalHandle {
    tx: Sender,
    always_use_relay: Arc<AtomicBool>,
}

impl SignalHandle {
    pub(crate) fn new(tx: Sender, always_use_relay: Arc<AtomicBool>) -> Self {
        Self {
            tx,
            always_use_relay,
        }
    }

    /// Replaces the relay list (`host[:port]`). Every entry is resolved here, on the caller's
    /// task, never on the server loop: concurrently, each bounded by a 3 s timeout; empty
    /// entries are dropped. If any entry does not resolve in time nothing is applied and
    /// [`SignalError::RelayServersRejected`] names them; otherwise the accepted list is
    /// returned and connections accepted afterwards use it.
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
        self.always_use_relay.store(on, Ordering::SeqCst);
        log::debug!("ALWAYS_USE_RELAY={}", if on { "Y" } else { "N" });
        Ok(())
    }

    fn ensure_running(&self) -> Result<(), SignalError> {
        if self.tx.is_closed() {
            return Err(SignalError::Stopped);
        }
        Ok(())
    }
}

/// A configured server that is not running yet.
pub struct SignalServer {
    cfg: SignalConfig,
    store: Arc<dyn PeerStore>,
    handle: SignalHandle,
    rx: Receiver,
}

impl SignalServer {
    /// Prepares a server that keeps peer registrations in `store`; nothing is bound or
    /// resolved until [`SignalServer::run`].
    ///
    /// The store is what binds a registered id to its device uuid across restarts (the
    /// UUID_MISMATCH anti id-hijack check); with a non-persistent store such as
    /// [`MemoryPeerStore`](crate::MemoryPeerStore), any uuid can claim any id after a process
    /// restart.
    pub fn new(cfg: SignalConfig, store: Arc<dyn PeerStore>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let always_use_relay = Arc::new(AtomicBool::new(cfg.always_use_relay));
        Self {
            cfg,
            store,
            handle: SignalHandle::new(tx, always_use_relay),
            rx,
        }
    }

    /// A handle for changing relay settings while the server runs; take it before `run`.
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
        let stop = shutdown.child_token();
        // Also stops the tasks if this future is dropped instead of awaited to the end.
        let _stop_on_drop = stop.clone().drop_guard();
        let tasks = Tasks::new(stop);
        let res = self.serve_until(ports, tasks.clone(), &shutdown).await;
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
            handle,
            rx,
        } = self;
        let serve = RendezvousServer::serve(
            cfg,
            ports,
            store,
            handle.tx,
            rx,
            handle.always_use_relay,
            tasks,
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

/// [`SignalServer::run`] for callers that never change relay settings at runtime.
pub async fn run(
    cfg: SignalConfig,
    store: Arc<dyn PeerStore>,
    shutdown: CancellationToken,
) -> Result<(), SignalError> {
    SignalServer::new(cfg, store).run(shutdown).await
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
}
