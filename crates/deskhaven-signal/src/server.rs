//! Embedding API (deskhaven code, not vendored): configuration, lifecycle and runtime control
//! of the signal server. Replaces upstream's command line, environment variables, process
//! signal handling and loopback console.

use crate::rendezvous_server::{Data, Receiver, RendezvousServer, Sender};
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
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio_util::sync::CancellationToken;

/// Upstream default of `MAX_PUNCH_REQUESTS`.
pub const DEFAULT_PUNCH_REQ_MAX: usize = 100_000;
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
    /// Relay address for LAN peers when `mask` is set (upstream `LOCAL_IP`). `None`: the
    /// detected local IP. A string because it stands in for a `relay_servers` entry.
    pub local_ip: Option<String>,
    /// Initial relay servers, `host[:port]` (upstream `-r`); entries that are empty or do not
    /// resolve are dropped. Change at runtime with [`SignalHandle::set_relay_servers`].
    pub relay_servers: Vec<String>,
    /// Force relayed connections (upstream `ALWAYS_USE_RELAY`). Change at runtime with
    /// [`SignalHandle::set_always_use_relay`].
    pub always_use_relay: bool,
    /// Bound of the in-memory punch request log, 0 disables it (upstream `MAX_PUNCH_REQUESTS`).
    pub punch_req_max: usize,
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
    #[error("UDP self-test failed")]
    SelfTest(#[source] BoxError),
    #[error("signal server failed")]
    Internal(#[source] BoxError),
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

    /// Replaces the relay list (same filtering as [`SignalConfig::relay_servers`]). Applied by
    /// the server loop; connections accepted afterwards use it. `Err(Stopped)` once `run` has
    /// returned.
    pub fn set_relay_servers(&self, servers: Vec<String>) -> Result<(), SignalError> {
        self.tx
            .send(Data::RelayServers0(servers))
            .map_err(|_| SignalError::Stopped)
    }

    /// Takes effect immediately for every connection.
    pub fn set_always_use_relay(&self, on: bool) {
        self.always_use_relay.store(on, Ordering::SeqCst);
        log::info!("ALWAYS_USE_RELAY={}", if on { "Y" } else { "N" });
    }
}

/// A configured server that is not running yet.
pub struct SignalServer {
    cfg: SignalConfig,
    handle: SignalHandle,
    rx: Receiver,
}

impl SignalServer {
    pub fn new(cfg: SignalConfig) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let always_use_relay = Arc::new(AtomicBool::new(cfg.always_use_relay));
        Self {
            cfg,
            handle: SignalHandle::new(tx, always_use_relay),
            rx,
        }
    }

    pub fn handle(&self) -> SignalHandle {
        self.handle.clone()
    }

    /// Serves on the caller's runtime until `shutdown` is cancelled (`Ok`) or a listener cannot
    /// be (re)created or the UDP self-test fails (`Err`). When it returns, every listener is
    /// closed and every task it spawned has finished.
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
        let Self { cfg, handle, rx } = self;
        let serve =
            RendezvousServer::serve(cfg, ports, handle.tx, rx, handle.always_use_relay, tasks);
        tokio::select! {
            res = serve => res,
            () = shutdown.cancelled() => {
                log::info!("signal server stopped");
                Ok(())
            }
        }
    }
}

/// [`SignalServer::run`] for callers that never change relay settings at runtime.
pub async fn run(cfg: SignalConfig, shutdown: CancellationToken) -> Result<(), SignalError> {
    SignalServer::new(cfg).run(shutdown).await
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
