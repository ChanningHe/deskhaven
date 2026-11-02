// Modified by deskhaven; see crates/deskhaven-signal/CHANGES.md (AGPL-3.0 §5a).
use crate::claimed_addr::{ClaimedAddr, ClaimedAddrs};
use crate::common::*;
use crate::control::ControlPlane;
use crate::log_limit::LogLimit;
use crate::peer::*;
use crate::server::{ClientIpResolver, Ports, SignalConfig, SignalError};
use crate::store::PeerStore;
use crate::tasks::Tasks;
use arc_swap::ArcSwap;
use hbb_common::{
    AddrMangle, ResultType, allow_err, bail,
    bytes::{Bytes, BytesMut},
    bytes_codec::BytesCodec,
    config,
    futures::future::join_all,
    futures_util::{
        sink::SinkExt,
        stream::{SplitSink, SplitStream, StreamExt},
    },
    log,
    protobuf::Message as _,
    rendezvous_proto::{
        register_pk_response::Result::{SERVER_ERROR, TOO_FREQUENT, UUID_MISMATCH},
        *,
    },
    tcp::FramedStream,
    tokio::{
        self,
        net::{TcpListener, TcpStream},
        sync::{Mutex, mpsc, watch},
        time::{
            Duration, Instant as TokioInstant, MissedTickBehavior, interval, interval_at,
            timeout_at,
        },
    },
    tokio_util::codec::Framed,
    try_into_v4,
    udp::FramedSocket,
};
use ipnetwork::Ipv4Network;
use sodiumoxide::crypto::sign;
use std::{
    collections::{HashMap, VecDeque},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

#[derive(Clone, Debug)]
pub(crate) enum Data {
    Msg(Box<RendezvousMessage>, SocketAddr),
    // deskhaven: an already resolved list (from `SignalHandle::set_relay_servers`) instead of
    // the console's comma-separated string.
    RelayServers0(RelayServers),
    // deskhaven: (list the health check ran on, healthy subset); applied only while that
    // list is still current.
    RelayServers(Arc<RelayServers>, RelayServers),
    // deskhaven: result of resolving the configured list (`relay_config`) off the loop;
    // applied only while that list is still current.
    RelayResolved {
        config: Arc<RelayServers>,
        accepted: RelayServers,
        rejected: RelayServers,
    },
    // deskhaven (M1-06): test-only fault injection into the server loop.
    #[cfg(any(test, feature = "testing"))]
    Inject(crate::server::Fault),
}

const REG_TIMEOUT: i64 = 30_000;
type TcpStreamSink = SplitSink<Framed<TcpStream, BytesCodec>, Bytes>;
type WsSink = SplitSink<tokio_tungstenite::WebSocketStream<TcpStream>, tungstenite::Message>;
enum Sink {
    TcpStream(TcpStreamSink),
    Ws(WsSink),
}

// deskhaven (M1-08): a controlled peer registered over a TCP / websocket connection (TCP-mode
// clients: proxy, websocket, UDP disabled). The connection task owns the sink; other tasks
// queue the peer's PunchHole / FetchLocalAddr / RequestRelay for it.
type StreamOutbox = (
    mpsc::Sender<RendezvousMessage>,
    mpsc::Receiver<RendezvousMessage>,
);
/// Connection-side state: the id registered over this connection and its queue.
#[derive(Default)]
struct StreamPeer {
    id: Option<String>,
    outbox: Option<StreamOutbox>,
}
/// Server-side entry (`stream_peers`, by id): where the connection is and its queue.
struct StreamPeerTx {
    addr: SocketAddr,
    tx: mpsc::Sender<RendezvousMessage>,
}
/// Messages waiting for one peer connection; a peer gets a few per connection attempt.
const STREAM_OUTBOX_LEN: usize = 32;
/// Heartbeat (empty frame) period on a registered connection. TCP-mode clients send nothing
/// once registered but answer each empty frame with one (rendezvous_mediator.rs `start_tcp`);
/// the answers keep the peer online (REG_TIMEOUT 30 s: three per period) and the connection
/// within IDLE_TIMEOUT, and the heartbeats keep the client's own timeout (1.5 x its 60 s
/// keep-alive) and reverse proxies' read timeouts (nginx: 60 s) from closing it.
const STREAM_HEARTBEAT: Duration = Duration::from_secs(10);
/// Bound of one send on a TCP / websocket connection (M1-08 review). A send only waits once the
/// client's receive window and our socket buffer are full, i.e. the client has stopped reading
/// for a while; frames are at most MAX_FRAME_LEN. Shorter than REG_TIMEOUT, so a stalled
/// registered peer goes offline sooner than a silent one, and it bounds how long an answer to a
/// stalled controller can hold up the answering peer's connection task (`send_to_tcp_sync`).
const STREAM_SEND_TIMEOUT: Duration = Duration::from_secs(10);

/// The next message queued for this connection's peer; pending while none is registered.
async fn next_outbound(outbox: &mut Option<StreamOutbox>) -> Option<RendezvousMessage> {
    match outbox {
        Some((_, rx)) => rx.recv().await,
        None => std::future::pending().await,
    }
}

// deskhaven (M1-08): the read half of a stream connection, so TCP and websocket share one read
// loop and its limits.
enum Reader {
    Tcp(SplitStream<Framed<TcpStream, BytesCodec>>),
    Ws(SplitStream<tokio_tungstenite::WebSocketStream<TcpStream>>),
}

impl Reader {
    /// The next frame's payload; `Some(None)` for a websocket message that is not binary
    /// (skipped like upstream, but it counts as activity); `None` once the connection is closed
    /// or broken, a frame over MAX_FRAME_LEN included.
    async fn next(&mut self) -> Option<Option<Bytes>> {
        match self {
            Self::Tcp(s) => s.next().await?.ok().map(|b| Some(b.freeze())),
            Self::Ws(s) => match s.next().await?.ok()? {
                tungstenite::Message::Binary(b) => Some(Some(b)),
                _ => Some(None),
            },
        }
    }
}

// deskhaven (M1-08): websocket messages and frames are bounded like TCP frames (tungstenite's
// defaults allow 64 MiB messages); the frame header is checked before its payload is read.
fn ws_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_FRAME_LEN))
        .max_frame_size(Some(MAX_FRAME_LEN))
}
pub(crate) type Sender = mpsc::UnboundedSender<Data>;
pub(crate) type Receiver = mpsc::UnboundedReceiver<Data>;
// deskhaven (M1-06): the ROTATION_RELAY_SERVER static moved into `AllowAll` (control.rs).
type RelayServers = Vec<String>;
const CHECK_RELAY_TIMEOUT: u64 = 3_000;
// deskhaven: hard cap on IP_BLOCKER entries (one per client IP seen in the last day); see
// `check_ip_block`.
const MAX_IP_BLOCK_ENTRIES: usize = 100_000;
// deskhaven (M1-05): RegisterPk key bounds. A RustDesk pk is an ed25519 public key; client
// uuids are machine ids (16-64 bytes in practice), so 128 leaves ample room.
const REGISTER_PK_LEN: usize = sign::PUBLICKEYBYTES;
const MAX_REGISTER_UUID_LEN: usize = 128;
// deskhaven (M1-05): dropped malformed RegisterPk are logged at most once a minute, with count.
static MALFORMED_PK_LOG: LogLimit = LogLimit::new(std::time::Duration::from_secs(60));
// deskhaven: the ALWAYS_USE_RELAY static became the per-instance `always_use_relay` field.

// deskhaven (M1-08): limits of the stream connections (TCP main / NAT test port, websocket;
// 07 §2.4). Upstream decoded frames without a size limit and gave each frame 30 s.
/// Largest frame (TCP) or message (websocket) accepted. The largest messages clients send to
/// these ports are PunchHoleRequest / PunchHoleSent / RelayResponse carrying a WebRTC SDP
/// offer / answer (base64 of the JSON SDP: a few KiB), OnlineRequest (one id per peer shown;
/// about 5000 ids fit) and, from M1-07, the KeyExchange reply (about 100 bytes); RegisterPk is
/// under 512 bytes (pk 32, uuid at most 128, id at most 253). 64 KiB leaves ten times the
/// room. HttpProxyRequest bodies (M5) may need more.
const MAX_FRAME_LEN: usize = 64 * 1024;
/// From accept to the first complete frame (websocket: handshake included). Well above the
/// client's READ_TIMEOUT (18 s): logged-in controllers and every TCP-mode controlled client
/// first wait that long for a KeyExchange (`secure_tcp`) from a server that sends none (upstream's
/// 30 s per frame). M1-07 sends the KeyExchange at once and may lower it.
const FIRST_FRAME_TIMEOUT_MS: u64 = 30_000;
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_millis(FIRST_FRAME_TIMEOUT_MS);
/// Between frames after the first (upstream's 30 s per frame).
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

// deskhaven (M1-08 review): bounds of the strings a client hands the server to pass on to another
// client (RequestRelay to the target; RelayResponse / PunchHoleSent / LocalAddr to the
// controller). A message over a bound is dropped and its connection closed. Forwarded
// RequestRelays wait in a peer's queue (STREAM_OUTBOX_LEN), so this also bounds that memory.
/// RequestRelay / RelayResponse `uuid`: clients send `Uuid::new_v4().to_string()` (36 chars).
const MAX_RELAY_UUID_LEN: usize = 64;
/// `relay_server` (`host:port`): a DNS name (at most 253) plus `:` and a 5-digit port; a
/// bracketed IPv6 literal is shorter.
const MAX_RELAY_SERVER_LEN: usize = 259;
/// RelayResponse / PunchHoleSent / LocalAddr `version`: the client's version string (e.g.
/// "1.4.1").
const MAX_VERSION_LEN: usize = 64;

/// Whether a message's passed-on strings are within their bounds (logged when not).
fn within_bounds(what: &str, uuid: &str, relay_server: &str, version: &str) -> bool {
    let ok = uuid.len() <= MAX_RELAY_UUID_LEN
        && relay_server.len() <= MAX_RELAY_SERVER_LEN
        && version.len() <= MAX_VERSION_LEN;
    if !ok {
        log::debug!(
            "dropped {what} with oversized fields (uuid {}, relay_server {}, version {} bytes)",
            uuid.len(),
            relay_server.len(),
            version.len()
        );
    }
    ok
}

// Store punch hole requests
use once_cell::sync::Lazy;
use tokio::sync::Mutex as TokioMutex; // differentiate if needed
#[derive(Clone)]
struct PunchReqEntry {
    tm: Instant,
    from_ip: String,
    // deskhaven: only read by the removed console command `pr`; the log is write-only until
    // a reader (e.g. an admin view) exists.
    #[allow(dead_code)]
    to_ip: String,
    to_id: String,
}
static PUNCH_REQS: Lazy<TokioMutex<VecDeque<PunchReqEntry>>> =
    Lazy::new(|| TokioMutex::new(VecDeque::new()));
const PUNCH_REQ_DEDUPE_SEC: u64 = 60;

// deskhaven: `max` comes from `SignalConfig::punch_req_max` (was the PUNCH_REQ_MAX static read
// from the MAX_PUNCH_REQUESTS env var).
async fn record_punch_req(max: usize, from_ip: String, to_ip: String, to_id: String) {
    if max == 0 {
        return;
    }
    let mut lock = PUNCH_REQS.lock().await;
    let mut dup = false;
    for e in lock.iter().rev().take(30) {
        // only check recent tail subset for speed
        if e.from_ip == from_ip && e.to_id == to_id {
            if e.tm.elapsed().as_secs() < PUNCH_REQ_DEDUPE_SEC {
                dup = true;
            }
            break;
        }
    }
    if !dup {
        if lock.len() >= max {
            lock.pop_front();
        }
        lock.push_back(PunchReqEntry {
            tm: Instant::now(),
            from_ip,
            to_ip,
            to_id,
        });
    }
}

#[derive(Clone)]
struct Inner {
    // deskhaven: `serial` removed with the loopback ConfigureUpdate backdoor (see handle_udp);
    // `version` / `software_url` (deprecated `-u`, SoftwareUpdate answers) removed.
    mask: Option<Ipv4Network>,
    local_ip: String,
    // deskhaven: always present, injected through `SignalConfig::secret_key`.
    sk: sign::SecretKey,
    punch_req_max: usize,
}

#[derive(Clone)]
pub(crate) struct RendezvousServer {
    tcp_punch: Arc<Mutex<HashMap<SocketAddr, Sink>>>,
    // deskhaven (M1-08): peers registered over TCP / websocket, by id.
    stream_peers: Arc<Mutex<HashMap<String, StreamPeerTx>>>,
    // deskhaven (M1-08): per-connection addresses of websocket clients behind a proxy.
    ws_addrs: ClaimedAddrs,
    // deskhaven (M1-08 review): decides whether a websocket client's proxy headers are trusted.
    ws_client_ip: Arc<dyn ClientIpResolver>,
    pm: PeerMap,
    tx: Sender,
    // deskhaven (M1-06): the relays handed out (resolved, health-checked subset), shared by the
    // server loop (the only writer) and every connection task (lock-free readers). Upstream
    // kept a plain `Arc` that each connection task cloned at accept (rs:1174-1182), so a list
    // change never reached connections that were already open.
    relay_servers: Arc<ArcSwap<RelayServers>>,
    relay_servers0: Arc<RelayServers>,
    // deskhaven: the configured relay list (initial config or last handle update); its
    // resolvable entries are `relay_servers0`.
    relay_config: Arc<RelayServers>,
    // deskhaven: configured entries that did not resolve on the last attempt, retried on the
    // timer while non-empty; `None` until the first attempt.
    relay_unresolved: Option<RelayServers>,
    // deskhaven: a resolution of `relay_config` is in flight (no overlapping retries).
    relay_resolving: bool,
    // deskhaven: `rendezvous_servers` removed with the ConfigureUpdate backdoor.
    inner: Arc<Inner>,
    always_use_relay: Arc<AtomicBool>,
    // deskhaven (M1-06): policy hooks (07 §3.2); only `pick_relay` is called so far.
    control: Arc<dyn ControlPlane>,
    tasks: Tasks,
}

enum LoopFailure {
    UdpSocket,
    Listener3,
    Listener2,
    Listener,
    // deskhaven (M1-06): `Fault::Error` injected by a test.
    #[cfg(any(test, feature = "testing"))]
    Injected,
}

impl RendezvousServer {
    // deskhaven: replaces upstream `start` / `start_with_bind` (own tokio runtime, env-var
    // arguments, own SIGTERM/INT/QUIT handling, loopback console, exiting the process). Runs on
    // the caller's runtime until a listener cannot be re-created or the UDP self-test fails;
    // the caller (`SignalServer::run`) handles cancellation. `serving` turns true once every
    // listener is bound (M1-06; the caller resets it when `run` ends).
    pub(crate) async fn serve(
        cfg: SignalConfig,
        ports: Ports,
        store: Arc<dyn PeerStore>,
        control: Arc<dyn ControlPlane>,
        tx: Sender,
        mut rx: Receiver,
        always_use_relay: Arc<AtomicBool>,
        tasks: Tasks,
        serving: &watch::Sender<bool>,
    ) -> Result<(), SignalError> {
        let key = cfg.licence_key();
        log::info!("Key: {}", key);
        let bind_addr = cfg.bind;
        let rmem = cfg.rmem;
        let mut rs = Self::new(&cfg, store, control, tx, always_use_relay, tasks)
            .await
            .map_err(SignalError::internal)?;
        // deskhaven (M1-05): the registry is in memory before anything is answered (row count
        // and duration are logged by the preload).
        rs.pm
            .preload(cfg.peer_preload_timeout)
            .await
            .map_err(SignalError::PeerPreload)?;
        let mut socket = bind_udp(bind_addr, ports.main, rmem).await?;
        let mut listener = bind_tcp(bind_addr, ports.main).await?;
        let mut listener2 = bind_tcp(bind_addr, ports.nat).await?;
        let mut listener3 = bind_tcp(bind_addr, ports.ws).await?;
        let test_addr = local_addr(&listener)?;
        log::info!("Listening on tcp/udp {}", test_addr);
        log::info!(
            "Listening on tcp {}, extra port for NAT test",
            local_addr(&listener2)?
        );
        log::info!("Listening on websocket {}", local_addr(&listener3)?);
        log::info!(
            "ALWAYS_USE_RELAY={}",
            if rs.always_use_relay.load(Ordering::SeqCst) {
                "Y"
            } else {
                "N"
            }
        );
        serving.send_replace(true);
        let main_task = async move {
            loop {
                log::info!("Start");
                match rs
                    .io_loop(
                        &mut rx,
                        &mut listener,
                        &mut listener2,
                        &mut listener3,
                        &mut socket,
                        &key,
                    )
                    .await
                {
                    LoopFailure::UdpSocket => {
                        drop(socket);
                        socket = bind_udp(bind_addr, ports.main, rmem).await?;
                    }
                    LoopFailure::Listener => {
                        drop(listener);
                        listener = bind_tcp(bind_addr, ports.main).await?;
                    }
                    LoopFailure::Listener2 => {
                        drop(listener2);
                        listener2 = bind_tcp(bind_addr, ports.nat).await?;
                    }
                    LoopFailure::Listener3 => {
                        drop(listener3);
                        listener3 = bind_tcp(bind_addr, ports.ws).await?;
                    }
                    #[cfg(any(test, feature = "testing"))]
                    LoopFailure::Injected => {
                        return Err(SignalError::internal("injected failure"));
                    }
                }
            }
        };
        tokio::select!(
            res = main_task => res,
            Err(err) = run_self_test(test_addr) => Err(SignalError::self_test(err)),
        )
    }

    // deskhaven: construction split out of `start_with_bind`; configuration comes from
    // `SignalConfig` instead of env vars. The deprecated `-s` serial and `-R`
    // rendezvous-servers are gone. No DNS here: the initial relay list starts out unresolved
    // and is resolved by the first timer tick in `io_loop`, i.e. after the listeners are bound
    // and off the loop (`on_tick`). An empty `local_ip` means auto-detect, like upstream
    // `get_arg_or`. The peer map persists to the injected store (M1-05).
    async fn new(
        cfg: &SignalConfig,
        store: Arc<dyn PeerStore>,
        control: Arc<dyn ControlPlane>,
        tx: Sender,
        always_use_relay: Arc<AtomicBool>,
        tasks: Tasks,
    ) -> ResultType<Self> {
        let pm = PeerMap::new(store, &tasks);
        let mask = cfg.mask;
        let local_ip = match (&mask, cfg.local_ip.as_deref().filter(|ip| !ip.is_empty())) {
            (None, _) => "".to_owned(),
            (Some(_), Some(local_ip)) => local_ip.to_owned(),
            (Some(_), None) => local_ip_address::local_ip()
                .map(|x| x.to_string())
                .unwrap_or_default(),
        };
        let relay_config: RelayServers = cfg
            .relay_servers
            .iter()
            .filter(|x| !x.is_empty())
            .cloned()
            .collect();
        let rs = Self {
            tcp_punch: Arc::new(Mutex::new(HashMap::new())),
            stream_peers: Default::default(),
            ws_addrs: Default::default(),
            ws_client_ip: cfg.ws_client_ip.clone(),
            pm,
            tx,
            relay_servers: Default::default(),
            relay_servers0: Default::default(),
            relay_unresolved: if relay_config.is_empty() {
                Some(Vec::new())
            } else {
                None
            },
            relay_config: Arc::new(relay_config),
            relay_resolving: false,
            inner: Arc::new(Inner {
                sk: cfg.secret_key.clone(),
                mask,
                local_ip,
                punch_req_max: cfg.punch_req_max,
            }),
            always_use_relay,
            control,
            tasks,
        };
        log::info!("mask: {:?}", rs.inner.mask);
        log::info!("local-ip: {:?}", rs.inner.local_ip);
        Ok(rs)
    }

    // deskhaven: the timer branch of `io_loop`, split out so tests can drive it. Prunes
    // IP_BLOCKER (upstream only pruned from the console's `ib`), starts the relay health check
    // (upstream), and (re)resolves the configured relay list while entries are unresolved.
    async fn on_tick(&mut self) {
        prune_ip_blocks(&mut *IP_BLOCKER.lock().await, Instant::now());
        if self.relay_servers0.len() > 1 {
            let rs = self.relay_servers0.clone();
            let tx = self.tx.clone();
            self.tasks.spawn(async move {
                check_relay_servers(rs, tx).await;
            });
        }
        let pending = self.relay_unresolved.as_ref().is_none_or(|x| !x.is_empty());
        if pending && !self.relay_resolving {
            self.relay_resolving = true;
            let config = self.relay_config.clone();
            let tx = self.tx.clone();
            self.tasks.spawn(async move {
                let (accepted, rejected) = resolve_servers(&config, RESOLVE_TIMEOUT).await;
                tx.send(Data::RelayResolved {
                    config,
                    accepted,
                    rejected,
                })
                .ok();
            });
        }
    }

    async fn io_loop(
        &mut self,
        rx: &mut Receiver,
        listener: &mut TcpListener,
        listener2: &mut TcpListener,
        listener3: &mut TcpListener,
        socket: &mut FramedSocket,
        key: &str,
    ) -> LoopFailure {
        let mut timer_check_relay = interval(Duration::from_millis(CHECK_RELAY_TIMEOUT));
        loop {
            tokio::select! {
                _ = timer_check_relay.tick() => {
                    self.on_tick().await;
                }
                Some(data) = rx.recv() => {
                    #[cfg(any(test, feature = "testing"))]
                    #[allow(clippy::panic)]
                    if let Data::Inject(fault) = &data {
                        match fault {
                            crate::server::Fault::Error => return LoopFailure::Injected,
                            crate::server::Fault::Panic => panic!("injected panic"),
                        }
                    }
                    self.handle_data(data, socket).await;
                }
                res = socket.next() => {
                    match res {
                        Some(Ok((bytes, addr))) => {
                            if let Err(err) = self.handle_udp(&bytes, addr.into(), socket, key).await {
                                log::error!("udp failure: {}", err);
                                return LoopFailure::UdpSocket;
                            }
                        }
                        Some(Err(err)) => {
                            log::error!("udp failure: {}", err);
                            return LoopFailure::UdpSocket;
                        }
                        None => {
                            // unreachable!() ?
                        }
                    }
                }
                res = listener2.accept() => {
                    match res {
                        Ok((stream, addr))  => {
                            stream.set_nodelay(true).ok();
                            self.handle_listener2(stream, addr).await;
                        }
                        Err(err) => {
                           log::error!("listener2.accept failed: {}", err);
                           return LoopFailure::Listener2;
                        }
                    }
                }
                res = listener3.accept() => {
                    match res {
                        Ok((stream, addr))  => {
                            stream.set_nodelay(true).ok();
                            self.handle_listener(stream, addr, key, true).await;
                        }
                        Err(err) => {
                           log::error!("listener3.accept failed: {}", err);
                           return LoopFailure::Listener3;
                        }
                    }
                }
                res = listener.accept() => {
                    match res {
                        Ok((stream, addr)) => {
                            stream.set_nodelay(true).ok();
                            self.handle_listener(stream, addr, key, false).await;
                        }
                       Err(err) => {
                           log::error!("listener.accept failed: {}", err);
                           return LoopFailure::Listener;
                       }
                    }
                }
            }
        }
    }

    // deskhaven: the `rx` branch of `io_loop`, split out so tests can drive it.
    async fn handle_data(&mut self, data: Data, socket: &mut FramedSocket) {
        match data {
            Data::Msg(msg, addr) => {
                allow_err!(socket.send(msg.as_ref(), addr).await);
            }
            Data::RelayServers0(rs) => {
                self.apply_relay_servers(rs);
            }
            // deskhaven: only apply a health-check result computed from the current list, so
            // a check still in flight when the list is replaced cannot restore old servers.
            Data::RelayServers(checked, rs) => {
                if Arc::ptr_eq(&checked, &self.relay_servers0) {
                    self.relay_servers.store(Arc::new(rs));
                }
            }
            #[cfg(any(test, feature = "testing"))]
            Data::Inject(_) => {} // handled in `io_loop`
            Data::RelayResolved {
                config,
                accepted,
                rejected,
            } => {
                self.relay_resolving = false;
                if Arc::ptr_eq(&config, &self.relay_config) {
                    self.apply_resolved_config(accepted, rejected);
                }
            }
        }
    }

    // deskhaven: applies a resolution of the configured list. An unchanged result keeps the
    // current health-checked subset; unresolved entries stay queued for the next tick.
    fn apply_resolved_config(&mut self, accepted: RelayServers, rejected: RelayServers) {
        if !rejected.is_empty() && self.relay_unresolved.as_ref() != Some(&rejected) {
            log::warn!("relay-servers not resolvable yet, retrying every 3 s: {rejected:?}");
        }
        self.relay_unresolved = Some(rejected);
        if accepted != *self.relay_servers0 {
            log::info!("relay-servers={accepted:?}");
            self.relay_servers0 = Arc::new(accepted);
            self.relay_servers.store(self.relay_servers0.clone());
        }
    }

    #[inline]
    async fn handle_udp(
        &mut self,
        bytes: &BytesMut,
        addr: SocketAddr,
        socket: &mut FramedSocket,
        _key: &str,
    ) -> ResultType<()> {
        if let Ok(msg_in) = RendezvousMessage::parse_from_bytes(bytes) {
            match msg_in.union {
                Some(rendezvous_message::Union::RegisterPeer(rp)) => {
                    // B registered
                    if !rp.id.is_empty() && rp.id.len() <= MAX_PEER_ID_LEN {
                        log::trace!("New peer registered: {:?} {:?}", &rp.id, &addr);
                        self.update_addr(rp.id, addr, socket).await?;
                        // deskhaven: the ConfigUpdate push (serial / rendezvous servers set by
                        // the removed loopback ConfigureUpdate) is gone.
                    }
                }
                Some(rendezvous_message::Union::RegisterPk(rk)) => {
                    // deskhaven (M1-08): checks and registration moved to `register_pk`, shared
                    // with TCP / websocket.
                    if let Some(res) = self.register_pk(rk, addr).await {
                        send_rk_res(socket, addr, res).await?;
                    }
                }
                Some(rendezvous_message::Union::PunchHoleRequest(_ph)) => {
                    // UDP PunchHoleRequest is intentionally unsupported.
                    // The supported client path sends PunchHoleRequest over TCP/WS.
                }
                Some(rendezvous_message::Union::PunchHoleSent(_phs)) => {
                    // UDP PunchHoleSent is intentionally unsupported to avoid UDP reflection/amplification
                }
                Some(rendezvous_message::Union::LocalAddr(_la)) => {
                    // UDP LocalAddr is intentionally unsupported to avoid UDP reflection/amplification
                }
                // deskhaven: ConfigureUpdate arm removed. It let any local process (UDP from
                // loopback, unauthenticated) set the serial and rendezvous-server list pushed to
                // every client, a backdoor like the removed console; it also did blocking DNS on
                // the server loop. Such config will come from the control plane.
                // deskhaven: SoftwareUpdate arm removed with the deprecated `-u` software-url
                // option (it only answered when that option was set).
                _ => {}
            }
        }
        Ok(())
    }

    // deskhaven (M1-08): the RegisterPk checks and registration, moved out of `handle_udp` so
    // RegisterPk over TCP / websocket gets the same ones. `None`: dropped without an answer;
    // otherwise the result to answer (upstream answered each case itself, over UDP).
    async fn register_pk(
        &mut self,
        rk: RegisterPk,
        addr: SocketAddr,
    ) -> Option<register_pk_response::Result> {
        if rk.uuid.is_empty() || rk.pk.is_empty() {
            return None;
        }
        // deskhaven: malformed keys are ignored like missing ones (above), before
        // memory or the store see them: a RustDesk pk is an ed25519 public key, and
        // an oversized uuid would only bloat every cached and stored row. Not
        // UUID_MISMATCH: that makes a real client give up its id.
        // Known limit (cf. 07 §2.4): RegisterPk is unauthenticated UDP, so a
        // spoofed source can still register (and persist) unclaimed ids; admission
        // (require_deploy / ControlPlane, M5) addresses that.
        if rk.pk.len() != REGISTER_PK_LEN || rk.uuid.len() > MAX_REGISTER_UUID_LEN {
            if let Some(n) = MALFORMED_PK_LOG.hit(Instant::now()) {
                log::debug!(
                    "dropped {n} malformed RegisterPk (last: {addr}, pk {} bytes, \
                     uuid {} bytes)",
                    rk.pk.len(),
                    rk.uuid.len()
                );
            }
            return None;
        }
        let id = rk.id;
        let ip = addr.ip().to_string();
        if id.len() < 6 || id.len() > MAX_PEER_ID_LEN {
            return Some(UUID_MISMATCH);
        } else if !self.check_ip_blocker(&ip, &id).await {
            return Some(TOO_FREQUENT);
        }
        // ControlPlane hook (M5): `control.register_pk(&RegisterInfo { id, uuid,
        // pk, ip, no_register_device })` goes here (inline, memory only): NotDeployed
        // -> NOT_DEPLOYED, UuidMismatch -> UUID_MISMATCH, before memory or the store
        // change. AllowAll answers Ok, so it is not called yet.
        let peer = self.pm.get_or(&id).await;
        let (changed, _ip_changed) = {
            let peer = peer.read().await;
            if peer.uuid.is_empty() {
                (true, false)
            } else {
                if peer.uuid == rk.uuid {
                    if peer.info.ip != ip && peer.pk != rk.pk {
                        log::warn!(
                            "Peer {} ip/pk mismatch: {}/{:?} vs {}/{:?}",
                            id,
                            ip,
                            rk.pk,
                            peer.info.ip,
                            peer.pk,
                        );
                        drop(peer);
                        return Some(UUID_MISMATCH);
                    }
                } else {
                    log::warn!(
                        "Peer {} uuid mismatch: {:?} vs {:?}",
                        id,
                        rk.uuid,
                        peer.uuid
                    );
                    drop(peer);
                    return Some(UUID_MISMATCH);
                }
                let ip_changed = peer.info.ip != ip;
                (
                    peer.uuid != rk.uuid || peer.pk != rk.pk || ip_changed,
                    ip_changed,
                )
            }
        };
        let mut req_pk = peer.read().await.reg_pk;
        if req_pk.1.elapsed().as_secs() > 6 {
            req_pk.0 = 0;
        } else if req_pk.0 > 2 {
            return Some(TOO_FREQUENT);
        }
        req_pk.0 += 1;
        req_pk.1 = Instant::now();
        peer.write().await.reg_pk = req_pk;
        // deskhaven: IP_CHANGES bookkeeping removed (only read by console `ic`).
        // deskhaven: the store write is queued, not awaited; refusing an unbound id
        // on a full queue is the only failure seen here (upstream ignored
        // update_pk's result). A store outage never reaches this path.
        if changed
            && self
                .pm
                .update_pk(id, peer, addr, rk.uuid, rk.pk, ip)
                .await
                .is_err()
        {
            return Some(SERVER_ERROR);
        }
        Some(register_pk_response::Result::OK)
    }

    #[inline]
    async fn handle_tcp(
        &mut self,
        bytes: &[u8],
        sink: &mut Option<Sink>,
        addr: SocketAddr,
        key: &str,
        ws: bool,
        conn: &mut StreamPeer,
    ) -> bool {
        if let Ok(msg_in) = RendezvousMessage::parse_from_bytes(bytes) {
            match msg_in.union {
                Some(rendezvous_message::Union::PunchHoleRequest(ph)) => {
                    // there maybe several attempt, so sink can be none
                    if let Some(sink) = sink.take() {
                        self.tcp_punch.lock().await.insert(try_into_v4(addr), sink);
                    }
                    allow_err!(self.handle_tcp_punch_hole_request(addr, ph, key, ws).await);
                    return true;
                }
                Some(rendezvous_message::Union::RequestRelay(mut rf)) => {
                    // deskhaven (M1-08 review): passed-on strings are bounded.
                    if !within_bounds("RequestRelay", &rf.uuid, &rf.relay_server, "") {
                        return false;
                    }
                    // there maybe several attempt, so sink can be none
                    if let Some(sink) = sink.take() {
                        self.tcp_punch.lock().await.insert(try_into_v4(addr), sink);
                    }
                    // ControlPlane hook (M5): `control.authorize(&ConnectReq { kind: Relay, .. })`
                    // goes here; Deny -> RelayResponse{refuse_reason}, Allow(grant) -> inject
                    // grant.permissions / audit_ref into the forwarded RequestRelay. AllowAll
                    // allows with an empty grant, so it is not called yet.
                    // deskhaven (M1-08): an unknown or offline (REG_TIMEOUT, as for punch holes)
                    // target is refused at once; upstream forwarded to stale peers and stayed
                    // silent for unknown ids, so the controller waited for its retries.
                    let Some(peer) = self.pm.get_in_memory(&rf.id).await else {
                        let refused = relay_refused(rf.uuid, REFUSE_ID_NOT_EXIST);
                        allow_err!(self.send_to_tcp_sync(refused, addr).await);
                        return true;
                    };
                    let (elapsed, peer_addr) = {
                        let r = peer.read().await;
                        (r.last_reg_time.elapsed().as_millis() as i64, r.socket_addr)
                    };
                    if elapsed >= REG_TIMEOUT {
                        let refused = relay_refused(rf.uuid, REFUSE_OFFLINE);
                        allow_err!(self.send_to_tcp_sync(refused, addr).await);
                        return true;
                    }
                    let mut msg_out = RendezvousMessage::new();
                    rf.socket_addr = AddrMangle::encode(addr).into();
                    // deskhaven (M1-08): the controller's credentials are for this server
                    // only; upstream forwarded them to the target, in clear over UDP.
                    rf.token.clear();
                    rf.licence_key.clear();
                    let id = rf.id.clone();
                    msg_out.set_request_relay(rf);
                    self.send_to_peer(&id, msg_out, peer_addr).await.ok();
                    return true;
                }
                Some(rendezvous_message::Union::RelayResponse(mut rr)) => {
                    // deskhaven (M1-08 review): passed-on strings are bounded.
                    if !within_bounds("RelayResponse", &rr.uuid, &rr.relay_server, &rr.version) {
                        return false;
                    }
                    let addr_b = AddrMangle::decode(&rr.socket_addr);
                    rr.socket_addr = Default::default();
                    let id = rr.id();
                    if !id.is_empty() {
                        let pk = self.get_pk(&rr.version, id.to_owned()).await;
                        rr.set_pk(pk);
                    }
                    let mut msg_out = RendezvousMessage::new();
                    if !rr.relay_server.is_empty() {
                        if self.is_lan(addr_b) {
                            // https://github.com/rustdesk/rustdesk-server/issues/24
                            rr.relay_server = self.inner.local_ip.clone();
                        } else if rr.relay_server == self.inner.local_ip {
                            rr.relay_server = self.get_relay_server(addr.ip(), addr_b.ip());
                        }
                    }
                    msg_out.set_relay_response(rr);
                    allow_err!(self.send_to_tcp_sync(msg_out, addr_b).await);
                }
                // deskhaven (M1-08 review): passed-on strings are bounded (both arms).
                Some(rendezvous_message::Union::PunchHoleSent(phs)) => {
                    if within_bounds("PunchHoleSent", "", &phs.relay_server, &phs.version) {
                        allow_err!(self.handle_hole_sent(phs, addr, None).await);
                    }
                }
                Some(rendezvous_message::Union::LocalAddr(la)) => {
                    if within_bounds("LocalAddr", "", &la.relay_server, &la.version) {
                        allow_err!(self.handle_local_addr(la, addr, None).await);
                    }
                }
                Some(rendezvous_message::Union::TestNatRequest(_tar)) => {
                    let mut msg_out = RendezvousMessage::new();
                    // deskhaven: no `cu` (ConfigUpdate) any more; see handle_udp.
                    let res = TestNatResponse {
                        port: addr.port() as _,
                        ..Default::default()
                    };
                    msg_out.set_test_nat_response(res);
                    Self::send_to_sink(sink, msg_out).await;
                }
                // deskhaven (M1-08): registration over TCP / websocket, for clients in proxy,
                // websocket or UDP-disabled mode (upstream answered RegisterPk with NOT_SUPPORT
                // and ignored RegisterPeer). Same checks and answers as over UDP; once
                // registered, the connection stays open as the peer's channel (heartbeats,
                // deliveries; see `handle_listener_inner`). A dropped RegisterPk (malformed)
                // or RegisterPeer (bad id) closes the connection.
                Some(rendezvous_message::Union::RegisterPk(rk)) => {
                    let id = rk.id.clone();
                    let Some(res) = self.register_pk(rk, addr).await else {
                        return false;
                    };
                    if res == register_pk_response::Result::OK {
                        self.bind_stream_peer(id, addr, conn).await;
                    }
                    Self::send_to_sink(sink, rk_response(res)).await;
                    return true;
                }
                // RegisterPeer proves nothing (no uuid), and over a stream the source IP
                // check of `refresh_peer` is weak (loopback exempt, websocket proxy headers),
                // so it must not move or bind a peer: it is always answered `request_pk`, and
                // only an accepted RegisterPk (uuid-checked) binds the connection. TCP-mode
                // clients never send it (they only send RegisterPk).
                Some(rendezvous_message::Union::RegisterPeer(rp)) => {
                    if rp.id.is_empty() || rp.id.len() > MAX_PEER_ID_LEN {
                        return false;
                    }
                    Self::send_to_sink(sink, register_peer_response(true)).await;
                    return true;
                }
                // ControlPlane hooks (M5): an `HttpProxyRequest` arm (`control.http_proxy`, only
                // on KeyExchange-secured connections, M1-07) and a `Hc` / HealthCheck arm
                // (`control.health_check`, keeping the connection open for `keep_alive`) go
                // here. Upstream ignores both messages, as AllowAll would.
                _ => {}
            }
        }
        false
    }

    #[inline]
    async fn update_addr(
        &mut self,
        id: String,
        socket_addr: SocketAddr,
        socket: &mut FramedSocket,
    ) -> ResultType<()> {
        let (request_pk, ip_change) = if let Some(old) = self.pm.get_in_memory(&id).await {
            let mut old = old.write().await;
            let ip = socket_addr.ip();
            let ip_change = if old.socket_addr.port() != 0 {
                ip != old.socket_addr.ip()
            } else {
                ip.to_string() != old.info.ip
            } && !ip.is_loopback();
            let request_pk = old.pk.is_empty() || ip_change;
            if !request_pk {
                old.socket_addr = socket_addr;
                old.last_reg_time = Instant::now();
            }
            let ip_change = if ip_change && old.reg_pk.0 <= 2 {
                Some(if old.socket_addr.port() == 0 {
                    old.info.ip.clone()
                } else {
                    old.socket_addr.to_string()
                })
            } else {
                None
            };
            (request_pk, ip_change)
        } else {
            (true, None)
        };
        if let Some(old) = ip_change {
            log::info!("IP change of {} from {} to {}", id, old, socket_addr);
        }
        socket
            .send(&register_peer_response(request_pk), socket_addr)
            .await
    }

    // deskhaven (M1-08): makes this stream connection the channel of peer `id`, now at `addr`:
    // the peer's address and liveness are refreshed (an unchanged RegisterPk skips
    // `update_pk`, and TCP-mode clients send no RegisterPeer), and deliveries to it go through
    // the connection (`send_to_peer`). A connection serves one id; a new one replaces it.
    async fn bind_stream_peer(&self, id: String, addr: SocketAddr, conn: &mut StreamPeer) {
        if let Some(old) = conn.id.take()
            && old != id
        {
            self.unbind_stream_peer(&old, addr).await;
        }
        if let Some(peer) = self.pm.get_in_memory(&id).await {
            let mut w = peer.write().await;
            w.socket_addr = addr;
            w.last_reg_time = Instant::now();
        }
        let tx = conn
            .outbox
            .get_or_insert_with(|| mpsc::channel(STREAM_OUTBOX_LEN))
            .0
            .clone();
        self.stream_peers
            .lock()
            .await
            .insert(id.clone(), StreamPeerTx { addr, tx });
        conn.id = Some(id);
    }

    // deskhaven (M1-08): a heartbeat answer from the peer's connection keeps it online, like a
    // RegisterPeer over UDP.
    async fn touch_stream_peer(&self, id: &str, addr: SocketAddr) {
        if let Some(peer) = self.pm.get_in_memory(id).await {
            let mut w = peer.write().await;
            if w.socket_addr == addr {
                w.last_reg_time = Instant::now();
            }
        }
    }

    // deskhaven (M1-08): the peer's connection closed: no more deliveries through it, and the
    // peer is offline at once while that connection is its address (it re-registers on a new
    // connection; nothing reaches it at the old address meanwhile).
    async fn unbind_stream_peer(&self, id: &str, addr: SocketAddr) {
        {
            let mut peers = self.stream_peers.lock().await;
            if peers.get(id).is_some_and(|p| p.addr == addr) {
                peers.remove(id);
            }
        }
        if let Some(peer) = self.pm.get_in_memory(id).await {
            let mut w = peer.write().await;
            if w.socket_addr == addr {
                w.last_reg_time = get_expired_time();
            }
        }
    }

    // deskhaven (M1-08): delivers to peer `id` through its TCP / websocket connection while that
    // is its current address, otherwise over UDP like upstream (`Data::Msg`). A full or closed
    // connection queue drops the message (logged), as a lost datagram would be.
    async fn send_to_peer(
        &self,
        id: &str,
        msg: RendezvousMessage,
        peer_addr: SocketAddr,
    ) -> ResultType<()> {
        let stream = self
            .stream_peers
            .lock()
            .await
            .get(id)
            .filter(|p| p.addr == peer_addr)
            .map(|p| p.tx.clone());
        match stream {
            Some(tx) => {
                if let Err(err) = tx.try_send(msg) {
                    log::debug!("message to {id} over its connection dropped: {err}");
                }
            }
            None => self.tx.send(Data::Msg(msg.into(), peer_addr))?,
        }
        Ok(())
    }

    #[inline]
    async fn handle_hole_sent<'a>(
        &mut self,
        phs: PunchHoleSent,
        addr: SocketAddr,
        socket: Option<&'a mut FramedSocket>,
    ) -> ResultType<()> {
        // punch hole sent from B, tell A that B is ready to be connected
        let addr_a = AddrMangle::decode(&phs.socket_addr);
        log::debug!(
            "{} punch hole response to {:?} from {:?}",
            if socket.is_none() { "TCP" } else { "UDP" },
            &addr_a,
            &addr
        );
        let mut msg_out = RendezvousMessage::new();
        let mut p = PunchHoleResponse {
            socket_addr: AddrMangle::encode(addr).into(),
            pk: self.get_pk(&phs.version, phs.id).await,
            relay_server: phs.relay_server.clone(),
            ..Default::default()
        };
        if let Ok(t) = phs.nat_type.enum_value() {
            p.set_nat_type(t);
        }
        msg_out.set_punch_hole_response(p);
        if let Some(socket) = socket {
            socket.send(&msg_out, addr_a).await?;
        } else {
            self.send_to_tcp(msg_out, addr_a).await;
        }
        Ok(())
    }

    #[inline]
    async fn handle_local_addr<'a>(
        &mut self,
        la: LocalAddr,
        addr: SocketAddr,
        socket: Option<&'a mut FramedSocket>,
    ) -> ResultType<()> {
        // relay local addrs of B to A
        let addr_a = AddrMangle::decode(&la.socket_addr);
        log::debug!(
            "{} local addrs response to {:?} from {:?}",
            if socket.is_none() { "TCP" } else { "UDP" },
            &addr_a,
            &addr
        );
        let mut msg_out = RendezvousMessage::new();
        let mut p = PunchHoleResponse {
            socket_addr: la.local_addr.clone(),
            pk: self.get_pk(&la.version, la.id).await,
            relay_server: la.relay_server,
            ..Default::default()
        };
        p.set_is_local(true);
        msg_out.set_punch_hole_response(p);
        if let Some(socket) = socket {
            socket.send(&msg_out, addr_a).await?;
        } else {
            self.send_to_tcp(msg_out, addr_a).await;
        }
        Ok(())
    }

    #[inline]
    async fn handle_punch_hole_request(
        &mut self,
        addr: SocketAddr,
        ph: PunchHoleRequest,
        key: &str,
        ws: bool,
    ) -> ResultType<(RendezvousMessage, Option<SocketAddr>)> {
        let mut ph = ph;
        if !key.is_empty() && ph.licence_key != key {
            log::warn!(
                "Authentication failed from {} for peer {} - invalid key",
                addr,
                ph.id
            );
            let mut msg_out = RendezvousMessage::new();
            msg_out.set_punch_hole_response(PunchHoleResponse {
                failure: punch_hole_response::Failure::LICENSE_MISMATCH.into(),
                ..Default::default()
            });
            return Ok((msg_out, None));
        }
        let id = ph.id;
        // ControlPlane hook (M5): `control.authorize(&ConnectReq { kind: Punch, .. })` goes here,
        // after the licence check and before the peer lookup; Deny -> PunchHoleResponse
        // {other_failure}, Allow(grant) -> inject grant.permissions / audit_ref into the
        // PunchHole / FetchLocalAddr below and remember the grant with A's sink in `tcp_punch`
        // (feedback in handle_hole_sent / handle_local_addr / RelayResponse). AllowAll allows
        // with an empty grant, so it is not called yet.
        // punch hole request from A, relay to B,
        // check if in same intranet first,
        // fetch local addrs if in same intranet.
        // because punch hole won't work if in the same intranet,
        // all routers will drop such self-connections.
        if let Some(peer) = self.pm.get(&id).await {
            let (elapsed, peer_addr) = {
                let r = peer.read().await;
                (r.last_reg_time.elapsed().as_millis() as i64, r.socket_addr)
            };
            if elapsed >= REG_TIMEOUT {
                let mut msg_out = RendezvousMessage::new();
                msg_out.set_punch_hole_response(PunchHoleResponse {
                    failure: punch_hole_response::Failure::OFFLINE.into(),
                    ..Default::default()
                });
                return Ok((msg_out, None));
            }

            // record punch hole request (from addr -> peer id/peer_addr)
            record_punch_req(
                self.inner.punch_req_max,
                try_into_v4(addr).ip().to_string(),
                try_into_v4(peer_addr).ip().to_string(),
                id.clone(),
            )
            .await;

            let mut msg_out = RendezvousMessage::new();
            let peer_is_lan = self.is_lan(peer_addr);
            let is_lan = self.is_lan(addr);
            let mut relay_server = self.get_relay_server(addr.ip(), peer_addr.ip());
            if self.always_use_relay.load(Ordering::SeqCst) || (peer_is_lan ^ is_lan) {
                if peer_is_lan {
                    // https://github.com/rustdesk/rustdesk-server/issues/24
                    relay_server = self.inner.local_ip.clone()
                }
                ph.nat_type = NatType::SYMMETRIC.into(); // will force relay
            }
            let same_intranet: bool = !ws
                && (peer_is_lan && is_lan || {
                    match (peer_addr, addr) {
                        (SocketAddr::V4(a), SocketAddr::V4(b)) => a.ip() == b.ip(),
                        (SocketAddr::V6(a), SocketAddr::V6(b)) => a.ip() == b.ip(),
                        _ => false,
                    }
                });
            let socket_addr = AddrMangle::encode(addr).into();
            if same_intranet {
                log::debug!(
                    "Fetch local addr {:?} {:?} request from {:?}",
                    id,
                    peer_addr,
                    addr
                );
                msg_out.set_fetch_local_addr(FetchLocalAddr {
                    socket_addr,
                    relay_server,
                    ..Default::default()
                });
            } else {
                log::debug!(
                    "Punch hole {:?} {:?} request from {:?}",
                    id,
                    peer_addr,
                    addr
                );
                msg_out.set_punch_hole(PunchHole {
                    socket_addr,
                    nat_type: ph.nat_type,
                    relay_server,
                    ..Default::default()
                });
            }
            Ok((msg_out, Some(peer_addr)))
        } else {
            let mut msg_out = RendezvousMessage::new();
            msg_out.set_punch_hole_response(PunchHoleResponse {
                failure: punch_hole_response::Failure::ID_NOT_EXIST.into(),
                ..Default::default()
            });
            Ok((msg_out, None))
        }
    }

    #[inline]
    async fn handle_online_request(
        &mut self,
        stream: &mut FramedStream,
        peers: Vec<String>,
    ) -> ResultType<()> {
        let mut states = BytesMut::zeroed((peers.len() + 7) / 8);
        for (i, peer_id) in peers.iter().enumerate() {
            if let Some(peer) = self.pm.get_in_memory(peer_id).await {
                let elapsed = peer.read().await.last_reg_time.elapsed().as_millis() as i64;
                // bytes index from left to right
                let states_idx = i / 8;
                let bit_idx = 7 - i % 8;
                if elapsed < REG_TIMEOUT {
                    states[states_idx] |= 0x01 << bit_idx;
                }
            }
        }

        let mut msg_out = RendezvousMessage::new();
        msg_out.set_online_response(OnlineResponse {
            states: states.into(),
            ..Default::default()
        });
        stream.send(&msg_out).await?;

        Ok(())
    }

    #[inline]
    async fn send_to_tcp(&mut self, msg: RendezvousMessage, addr: SocketAddr) {
        let mut tcp = self.tcp_punch.lock().await.remove(&try_into_v4(addr));
        self.tasks.spawn(async move {
            Self::send_to_sink(&mut tcp, msg).await;
        });
    }

    #[inline]
    async fn send_to_sink(sink: &mut Option<Sink>, msg: RendezvousMessage) {
        if let Ok(bytes) = msg.write_to_bytes() {
            Self::send_bytes_to_sink(sink, Bytes::from(bytes)).await;
        }
    }

    // deskhaven (M1-08): an empty frame (websocket: empty binary message) to a registered peer.
    async fn send_heartbeat(sink: &mut Option<Sink>) {
        Self::send_bytes_to_sink(sink, Bytes::new()).await;
    }

    // deskhaven (M1-08): the sending half of `send_to_sink`, split out for `send_heartbeat`.
    // deskhaven (M1-08 review): every send is bounded by STREAM_SEND_TIMEOUT (upstream: none, so
    // a client that stopped reading blocked the sending task for good once the socket buffers
    // were full). A failed or timed-out send drops the sink (`None`): nothing more is sent on
    // that connection, and its own task ends a registered connection (`handle_listener_inner`).
    async fn send_bytes_to_sink(sink: &mut Option<Sink>, bytes: Bytes) {
        let Some(s) = sink.as_mut() else {
            return;
        };
        let sent = match s {
            Sink::TcpStream(s) => matches!(
                tokio::time::timeout(STREAM_SEND_TIMEOUT, s.send(bytes)).await,
                Ok(Ok(()))
            ),
            Sink::Ws(ws) => matches!(
                tokio::time::timeout(
                    STREAM_SEND_TIMEOUT,
                    ws.send(tungstenite::Message::Binary(bytes))
                )
                .await,
                Ok(Ok(()))
            ),
        };
        if !sent {
            log::debug!("send to a stream connection failed or timed out, closing it");
            *sink = None;
        }
    }

    #[inline]
    async fn send_to_tcp_sync(
        &mut self,
        msg: RendezvousMessage,
        addr: SocketAddr,
    ) -> ResultType<()> {
        let mut sink = self.tcp_punch.lock().await.remove(&try_into_v4(addr));
        Self::send_to_sink(&mut sink, msg).await;
        Ok(())
    }

    #[inline]
    async fn handle_tcp_punch_hole_request(
        &mut self,
        addr: SocketAddr,
        ph: PunchHoleRequest,
        key: &str,
        ws: bool,
    ) -> ResultType<()> {
        let id = ph.id.clone();
        let (msg, to_addr) = self.handle_punch_hole_request(addr, ph, key, ws).await?;
        if let Some(addr) = to_addr {
            // deskhaven (M1-08): through the target's connection if it registered over one.
            self.send_to_peer(&id, msg, addr).await?;
        } else {
            self.send_to_tcp_sync(msg, addr).await?;
        }
        Ok(())
    }

    #[inline]
    #[allow(dead_code)] // upstream disabled the UDP punch path (see handle_udp)
    async fn handle_udp_punch_hole_request(
        &mut self,
        addr: SocketAddr,
        ph: PunchHoleRequest,
        key: &str,
    ) -> ResultType<()> {
        let (msg, to_addr) = self.handle_punch_hole_request(addr, ph, key, false).await?;
        self.tx.send(Data::Msg(
            msg.into(),
            match to_addr {
                Some(addr) => addr,
                None => addr,
            },
        ))?;
        Ok(())
    }

    // deskhaven: body moved to the free function `check_ip_block` (testable, with a cap).
    async fn check_ip_blocker(&self, ip: &str, id: &str) -> bool {
        check_ip_block(&mut *IP_BLOCKER.lock().await, ip, id, MAX_IP_BLOCK_ENTRIES)
    }

    // deskhaven: was `parse_relay_servers(&str)`. Takes a list that is already resolved and
    // validated (`resolve_servers`, off the server loop) instead of a comma-separated string
    // resolved with blocking DNS here; it replaces the configured list, so nothing is left to
    // retry. The loopback console (`check_cmd`) that followed here was removed:
    // unauthenticated, and its `aur` command overwrote the relay list with "Y"/"N".
    fn apply_relay_servers(&mut self, relay_servers: RelayServers) {
        self.relay_config = Arc::new(relay_servers.clone());
        self.relay_unresolved = Some(Vec::new());
        self.relay_servers0 = Arc::new(relay_servers);
        self.relay_servers.store(self.relay_servers0.clone());
    }
}

// deskhaven: upstream `check_ip_blocker` body, unchanged except for the cap: when the map
// already holds `max_entries` IPs, a new IP is refused (TOO_FREQUENT) without an insert, so the
// map stays bounded between prunes. Known IPs keep their normal accounting.
fn check_ip_block(lock: &mut IpBlockMap, ip: &str, id: &str, max_entries: usize) -> bool {
    let now = Instant::now();
    if let Some(old) = lock.get_mut(ip) {
        let counter = &mut old.0;
        if counter.1.elapsed().as_secs() > IP_BLOCK_DUR {
            counter.0 = 0;
        } else if counter.0 > 30 {
            return false;
        }
        counter.0 += 1;
        counter.1 = now;

        let counter = &mut old.1;
        let is_new = counter.0.get(id).is_none();
        if counter.1.elapsed().as_secs() > DAY_SECONDS {
            counter.0.clear();
        } else if counter.0.len() > 300 {
            return !is_new;
        }
        if is_new {
            counter.0.insert(id.to_owned());
        }
        counter.1 = now;
    } else if lock.len() >= max_entries {
        return false;
    } else {
        lock.insert(ip.to_owned(), ((0, now), (Default::default(), now)));
    }
    true
}

impl RendezvousServer {
    // deskhaven (M1-06): the choice is the control plane's (`AllowAll` keeps upstream's
    // round-robin), among the relays currently shared by every connection.
    fn get_relay_server(&self, pa: IpAddr, pb: IpAddr) -> String {
        self.control
            .pick_relay(pa, pb, &self.relay_servers.load())
            .unwrap_or_default()
    }

    async fn handle_listener2(&self, stream: TcpStream, addr: SocketAddr) {
        let mut rs = self.clone();
        // deskhaven: loopback connections are no longer diverted to the removed console; they
        // get the same NAT test / online query handling as everyone else.
        let mut stream = FramedStream::from(stream, addr);
        // deskhaven (M1-08): same frame and first-frame limits as the main port (was 30 s and
        // no size limit).
        stream.codec_mut().set_max_packet_length(MAX_FRAME_LEN);
        self.tasks.spawn(async move {
            let mut stream = stream;
            if let Some(Ok(bytes)) = stream.next_timeout(FIRST_FRAME_TIMEOUT_MS).await
                && let Ok(msg_in) = RendezvousMessage::parse_from_bytes(&bytes)
            {
                match msg_in.union {
                    Some(rendezvous_message::Union::TestNatRequest(_)) => {
                        let mut msg_out = RendezvousMessage::new();
                        msg_out.set_test_nat_response(TestNatResponse {
                            port: addr.port() as _,
                            ..Default::default()
                        });
                        stream.send(&msg_out).await.ok();
                    }
                    Some(rendezvous_message::Union::OnlineRequest(or)) => {
                        allow_err!(rs.handle_online_request(&mut stream, or.peers).await);
                    }
                    _ => {}
                }
            }
        });
    }

    async fn handle_listener(&self, stream: TcpStream, addr: SocketAddr, key: &str, ws: bool) {
        log::debug!("Tcp connection from {:?}, ws: {}", addr, ws);
        let mut rs = self.clone();
        let key = key.to_owned();
        self.tasks.spawn(async move {
            allow_err!(rs.handle_listener_inner(stream, addr, &key, ws).await);
        });
    }

    #[inline]
    async fn handle_listener_inner(
        &mut self,
        stream: TcpStream,
        mut addr: SocketAddr,
        key: &str,
        ws: bool,
    ) -> ResultType<()> {
        // deskhaven (M1-08): the first complete frame (websocket: handshake included) must
        // arrive within FIRST_FRAME_TIMEOUT of the accept, later ones within IDLE_TIMEOUT of the
        // previous one, and a frame over MAX_FRAME_LEN ends the connection as soon as its
        // header is read. Upstream gave every frame 30 s, the handshake no limit at all and
        // frames no size limit.
        let first_frame = TokioInstant::now() + FIRST_FRAME_TIMEOUT;
        let mut sink;
        let mut reader;
        // deskhaven (M1-08): held until the connection ends (websocket behind a proxy).
        let mut _claim: Option<ClaimedAddr> = None;
        if ws {
            use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
            // deskhaven (M1-08 review): upstream trusted X-Real-IP / X-Forwarded-For from
            // anyone who reached this port (https://github.com/rustdesk/rustdesk-server/issues/634),
            // so any client could pick its IP (per-IP limits, the anti id-hijack IP check,
            // loopback exemptions). The embedder's resolver decides now (deskhaven:
            // `security.trusted_proxies` with the HTTP API's X-Forwarded-For rule); by default
            // headers are ignored. A header-derived IP gets port 0, replaced by a claimed port
            // below.
            let resolver = self.ws_client_ip.clone();
            let peer = addr;
            let callback = |req: &Request, response: Response| {
                let ip = resolver.client_ip(peer.ip(), req.headers());
                if ip != peer.ip() && ip != peer.ip().to_canonical() {
                    addr = SocketAddr::new(ip, 0);
                }
                Ok(response)
            };
            let handshake = tokio_tungstenite::accept_hdr_async_with_config(
                stream,
                callback,
                Some(ws_config()),
            );
            let (a, b) = timeout_at(first_frame, handshake).await??.split();
            // deskhaven (M1-08): port 0 means the IP came from a proxy header; every client
            // behind that proxy IP had the same `ip:0` and they overwrote each other in
            // `tcp_punch` / as registered peers. Each connection holds its own port instead.
            if addr.port() == 0 {
                let Some(claim) = self.ws_addrs.claim(addr.ip()) else {
                    bail!("no free websocket address for {}", addr.ip());
                };
                addr = claim.addr();
                _claim = Some(claim);
            }
            sink = Some(Sink::Ws(a));
            reader = Reader::Ws(b);
        } else {
            let mut codec = BytesCodec::new();
            codec.set_max_packet_length(MAX_FRAME_LEN);
            let (a, b) = Framed::new(stream, codec).split();
            sink = Some(Sink::TcpStream(a));
            reader = Reader::Tcp(b);
        }
        let mut deadline = first_frame;
        // deskhaven (M1-08): a peer registered over this connection (`conn`) gets heartbeats
        // and the messages queued for it; an empty frame from it answers a heartbeat.
        let mut conn = StreamPeer::default();
        // First tick one period after the accept (not tied to FIRST_FRAME_TIMEOUT, which may
        // exceed IDLE_TIMEOUT - STREAM_HEARTBEAT); a peer registered later gets one at once.
        let mut heartbeat = interval_at(TokioInstant::now() + STREAM_HEARTBEAT, STREAM_HEARTBEAT);
        heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                res = timeout_at(deadline, reader.next()) => {
                    let Ok(Some(frame)) = res else { break };
                    deadline = TokioInstant::now() + IDLE_TIMEOUT;
                    let Some(bytes) = frame else { continue };
                    if bytes.is_empty()
                        && let Some(id) = &conn.id
                    {
                        self.touch_stream_peer(id, addr).await;
                    } else if !self
                        .handle_tcp(&bytes, &mut sink, addr, key, ws, &mut conn)
                        .await
                    {
                        break;
                    }
                }
                Some(msg) = next_outbound(&mut conn.outbox) => {
                    Self::send_to_sink(&mut sink, msg).await;
                }
                _ = heartbeat.tick(), if conn.id.is_some() => {
                    Self::send_heartbeat(&mut sink).await;
                }
            }
            // deskhaven (M1-08 review): a registered connection whose sink is gone (a send
            // failed or timed out) can no longer serve its peer.
            if conn.outbox.is_some() && sink.is_none() {
                break;
            }
        }
        if let Some(id) = &conn.id {
            self.unbind_stream_peer(id, addr).await;
        }
        if sink.is_none() {
            self.tcp_punch.lock().await.remove(&try_into_v4(addr));
        }
        log::debug!("Tcp connection from {:?} closed", addr);
        Ok(())
    }

    #[inline]
    async fn get_pk(&mut self, version: &str, id: String) -> Bytes {
        if version.is_empty() {
            Bytes::new()
        } else {
            match self.pm.get(&id).await {
                Some(peer) => {
                    let pk = peer.read().await.pk.clone();
                    sign::sign(
                        &hbb_common::rendezvous_proto::IdPk {
                            id,
                            pk,
                            ..Default::default()
                        }
                        .write_to_bytes()
                        .unwrap_or_default(),
                        &self.inner.sk,
                    )
                    .into()
                }
                _ => Bytes::new(),
            }
        }
    }

    // deskhaven: `get_server_sk` (key from `-k`: base64 secret key, arbitrary string, or
    // `-`/`_`/empty = read or create `id_ed25519` in the CWD) removed. The key is injected via
    // `SignalConfig::secret_key`; the licence key is its public key (`SignalConfig::licence_key`).

    #[inline]
    fn is_lan(&self, addr: SocketAddr) -> bool {
        if let Some(network) = &self.inner.mask {
            match addr {
                SocketAddr::V4(v4_socket_addr) => {
                    return network.contains(*v4_socket_addr.ip());
                }

                SocketAddr::V6(v6_socket_addr) => {
                    if let Some(v4_addr) = v6_socket_addr.ip().to_ipv4() {
                        return network.contains(v4_addr);
                    }
                }
            }
        }
        false
    }
}

async fn check_relay_servers(rs0: Arc<RelayServers>, tx: Sender) {
    let mut futs = Vec::new();
    let rs = Arc::new(Mutex::new(Vec::new()));
    for x in rs0.iter() {
        let mut host = x.to_owned();
        if !host.contains(':') {
            host = format!("{}:{}", host, config::RELAY_PORT);
        }
        let rs = rs.clone();
        let x = x.clone();
        // deskhaven: probes run concurrently inside the (tracked) caller task instead of as
        // detached `tokio::spawn` tasks, so none outlives the server.
        futs.push(async move {
            if FramedStream::new(&host, None, CHECK_RELAY_TIMEOUT)
                .await
                .is_ok()
            {
                rs.lock().await.push(x);
            }
        });
    }
    join_all(futs).await;
    log::debug!("check_relay_servers");
    let rs = std::mem::take(&mut *rs.lock().await);
    if !rs.is_empty() {
        tx.send(Data::RelayServers(rs0, rs)).ok();
    }
}

// deskhaven: upstream pruned IP_BLOCKER only from the removed console command `ib`, so it grew
// by one entry per client IP forever. Same predicate as `ib` (keep while the 60 s register
// counter or the daily id set is live), now run on the relay-check timer; `now` is a parameter
// for testing.
fn prune_ip_blocks(map: &mut IpBlockMap, now: Instant) {
    map.retain(|_, (counter, ids)| {
        now.saturating_duration_since(counter.1).as_secs() <= IP_BLOCK_DUR
            || now.saturating_duration_since(ids.1).as_secs() <= DAY_SECONDS
    });
}

// deskhaven: upstream spawned this and exited the process on failure (TEST_HBBS=no
// disabled it, TEST_HBBS=<addr> overrode the address). It is now always on, against the main
// listener, and a failure ends `serve` with `SignalError::SelfTest`.
async fn run_self_test(test_addr: SocketAddr) -> ResultType<()> {
    let Err(err) = test_hbbs(test_addr).await else {
        return Ok(());
    };
    if !(test_addr.is_ipv6() && test_addr.ip().is_unspecified()) {
        log::error!("Failed to run hbbs test with {test_addr}: {err}");
        return Err(err);
    }
    let mut test_addr = test_addr;
    test_addr.set_ip(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    test_hbbs(test_addr).await.inspect_err(|err| {
        log::error!("Failed to run hbbs test with {test_addr}: {err}");
    })
}

// temp solution to solve udp socket failure
async fn test_hbbs(addr: SocketAddr) -> ResultType<()> {
    let mut addr = addr;
    if addr.ip().is_unspecified() {
        addr.set_ip(if addr.is_ipv4() {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        } else {
            IpAddr::V6(Ipv6Addr::LOCALHOST)
        });
    }

    let mut socket = FramedSocket::new(config::Config::get_any_listen_addr(addr.is_ipv4())).await?;
    let mut msg_out = RendezvousMessage::new();
    msg_out.set_register_peer(RegisterPeer {
        id: "(:test_hbbs:)".to_owned(),
        ..Default::default()
    });
    let mut last_time_recv = Instant::now();

    let mut timer = interval(Duration::from_secs(1));
    loop {
        tokio::select! {
          _ = timer.tick() => {
              if last_time_recv.elapsed().as_secs() > 12 {
                  bail!("Timeout of test_hbbs");
              }
              socket.send(&msg_out, addr).await?;
          }
          Some(Ok((bytes, _))) = socket.next() => {
              if let Ok(msg_in) = RendezvousMessage::parse_from_bytes(&bytes) {
                 log::trace!("Recv {:?} of test_hbbs", msg_in);
                 last_time_recv = Instant::now();
              }
          }
        }
    }
}

// deskhaven (M1-08): refusal texts of RelayResponse. Clients show `refuse_reason` as the error
// (client.rs:1838-1839); these are the texts (and translation keys) the client itself uses for
// PunchHoleResponse ID_NOT_EXIST / OFFLINE.
const REFUSE_ID_NOT_EXIST: &str = "ID does not exist";
const REFUSE_OFFLINE: &str = "Remote desktop is offline";

// deskhaven (M1-08): a RelayResponse refusing the controller's RequestRelay `uuid`.
fn relay_refused(uuid: String, reason: &str) -> RendezvousMessage {
    let mut msg_out = RendezvousMessage::new();
    msg_out.set_relay_response(RelayResponse {
        uuid,
        refuse_reason: reason.to_owned(),
        ..Default::default()
    });
    msg_out
}

#[inline]
async fn send_rk_res(
    socket: &mut FramedSocket,
    addr: SocketAddr,
    res: register_pk_response::Result,
) -> ResultType<()> {
    socket.send(&rk_response(res), addr).await
}

// deskhaven (M1-08): the answers `send_rk_res` / `update_addr` send over UDP, as messages that
// TCP / websocket connections send too.
fn rk_response(res: register_pk_response::Result) -> RendezvousMessage {
    let mut msg_out = RendezvousMessage::new();
    msg_out.set_register_pk_response(RegisterPkResponse {
        result: res.into(),
        ..Default::default()
    });
    msg_out
}

fn register_peer_response(request_pk: bool) -> RendezvousMessage {
    let mut msg_out = RendezvousMessage::new();
    msg_out.set_register_peer_response(RegisterPeerResponse {
        request_pk,
        ..Default::default()
    });
    msg_out
}

// deskhaven (M1-08): `reuse = false` (upstream `true` set SO_REUSEPORT and SO_REUSEADDR, so a
// second instance shared the port). UDP needs neither for a quick restart.
async fn create_udp_listener(
    bind_addr: Option<IpAddr>,
    port: i32,
    rmem: usize,
) -> ResultType<FramedSocket> {
    if let Some(bind_addr) = bind_addr {
        let addr = SocketAddr::new(bind_addr, port as _);
        return FramedSocket::new_reuse(&addr, false, rmem).await;
    }
    let addr = SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port as _);
    if let Ok(s) = FramedSocket::new_reuse(&addr, false, rmem).await {
        log::debug!("listen on udp {:?}", s.local_addr());
        return Ok(s);
    }
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port as _);
    let s = FramedSocket::new_reuse(&addr, false, rmem).await?;
    log::debug!("listen on udp {:?}", s.local_addr());
    Ok(s)
}

#[inline]
async fn create_tcp_listener(bind_addr: Option<IpAddr>, port: i32) -> ResultType<TcpListener> {
    let s = listen_tcp(bind_addr, port as _).await?;
    log::debug!("listen on tcp {:?}", s.local_addr());
    Ok(s)
}

// deskhaven: bind wrappers naming the failed port in the error (upstream propagated `?`).
async fn bind_udp(
    bind_addr: Option<IpAddr>,
    port: u16,
    rmem: usize,
) -> Result<FramedSocket, SignalError> {
    create_udp_listener(bind_addr, port.into(), rmem)
        .await
        .map_err(|err| SignalError::bind("udp", port, err))
}

async fn bind_tcp(bind_addr: Option<IpAddr>, port: u16) -> Result<TcpListener, SignalError> {
    create_tcp_listener(bind_addr, port.into())
        .await
        .map_err(|err| SignalError::bind("tcp", port, err))
}

fn local_addr(listener: &TcpListener) -> Result<SocketAddr, SignalError> {
    listener.local_addr().map_err(SignalError::internal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hbb_common::timeout;

    fn memory_store() -> Arc<dyn PeerStore> {
        Arc::new(crate::store::MemoryPeerStore::new())
    }

    #[hbb_common::tokio::test]
    async fn udp_listener_uses_bind_address() {
        let bind_addr = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let socket = create_udp_listener(Some(bind_addr), 0, 0).await.unwrap();
        assert_eq!(socket.local_addr().unwrap().ip(), bind_addr);
    }

    // deskhaven: the bound comes from `SignalConfig::punch_req_max` (upstream read the
    // MAX_PUNCH_REQUESTS env var); both the default and 0 (recording disabled) are checked.
    #[hbb_common::tokio::test]
    async fn punch_reqs_bounded() {
        let before = PUNCH_REQS.lock().await.len();
        record_punch_req(
            0,
            "1.2.3.4".to_owned(),
            "5.6.7.8".to_owned(),
            "p".to_owned(),
        )
        .await;
        assert_eq!(
            PUNCH_REQS.lock().await.len(),
            before,
            "max 0 records nothing"
        );

        let max = crate::DEFAULT_PUNCH_REQ_MAX;
        // 31 ids keep every request just outside the 30-entry dedupe window
        for i in 0..max * 2 {
            record_punch_req(
                max,
                "1.2.3.4".to_owned(),
                "5.6.7.8".to_owned(),
                format!("peer{}", i % 31),
            )
            .await;
        }
        let lock = PUNCH_REQS.lock().await;
        assert_eq!(lock.len(), max);
        // the oldest half is evicted and the newest kept
        assert_eq!(lock.front().unwrap().to_id, format!("peer{}", max % 31));
        assert_eq!(
            lock.back().unwrap().to_id,
            format!("peer{}", (max * 2 - 1) % 31)
        );
    }

    // deskhaven (M1-03): relay settings changed through `SignalHandle` (replacing the removed
    // console `rs` / `aur` commands) reach relay selection and the punch-hole decision.
    // TEST-NET-1 addresses (RFC 5737): never reachable, so health probes never succeed.
    const PEER_ID: &str = "relay-test-peer";
    const RELAY_INIT: &str = "192.0.2.9:21117";
    const RELAY_A: &str = "192.0.2.1:21117";
    const RELAY_B: &str = "192.0.2.2:21117";
    const RELAY_C: &str = "192.0.2.3:21117";
    const RELAY_D: &str = "192.0.2.4:21117";
    /// Rejected without DNS (invalid port).
    const UNRESOLVABLE: &str = "bad host:x";

    struct Fixture {
        rs: RendezvousServer,
        handle: crate::SignalHandle,
        tx: Sender,
        rx: Receiver,
        socket: FramedSocket,
        key: String,
    }

    async fn fixture_with(relay_servers: &[&str]) -> Fixture {
        let cfg = crate::SignalConfig {
            relay_servers: relay_servers.iter().map(|s| (*s).to_owned()).collect(),
            punch_req_max: 0, // keep the process-wide PUNCH_REQS untouched
            ..crate::SignalConfig::new(sign::gen_keypair().1)
        };
        let (tx, rx) = mpsc::unbounded_channel();
        let always_use_relay = Arc::new(AtomicBool::new(cfg.always_use_relay));
        let (_, serving) = watch::channel(false);
        let handle = crate::SignalHandle::new(tx.clone(), always_use_relay.clone(), serving);
        let tasks = crate::tasks::Tasks::new(Default::default());
        let control = Arc::new(crate::AllowAll::new());
        let rs = RendezvousServer::new(
            &cfg,
            memory_store(),
            control,
            tx.clone(),
            always_use_relay,
            tasks,
        )
        .await
        .unwrap();
        let socket = create_udp_listener(Some(IpAddr::V4(Ipv4Addr::LOCALHOST)), 0, 0)
            .await
            .unwrap();
        let key = cfg.licence_key();
        Fixture {
            rs,
            handle,
            tx,
            rx,
            socket,
            key,
        }
    }

    /// A fixture whose initial relay list has been resolved (first timer tick).
    async fn fixture() -> Fixture {
        let mut f = fixture_with(&[RELAY_INIT]).await;
        f.tick_and_resolve().await;
        f
    }

    impl Fixture {
        /// Delivers queued handle updates the way `io_loop` does.
        async fn pump(&mut self) {
            while let Ok(data) = self.rx.try_recv() {
                self.rs.handle_data(data, &mut self.socket).await;
            }
        }

        /// Runs one timer tick and delivers everything up to the resulting relay resolution.
        async fn tick_and_resolve(&mut self) {
            self.rs.on_tick().await;
            loop {
                let data = timeout(5_000, self.rx.recv()).await.unwrap().unwrap();
                let resolved = matches!(data, Data::RelayResolved { .. });
                self.rs.handle_data(data, &mut self.socket).await;
                if resolved {
                    return;
                }
            }
        }

        fn relays(&self) -> Vec<String> {
            (**self.rs.relay_servers.load()).clone()
        }

        /// A pick is always one of the current relays.
        fn assert_picks_from_relays(&self) {
            let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
            let relays = self.relays();
            for _ in 0..relays.len() * 2 {
                let pick = self.rs.get_relay_server(ip, ip);
                assert!(relays.contains(&pick), "{pick} not in {relays:?}");
            }
        }

        async fn online_peer(&self) -> SocketAddr {
            let peer_addr: SocketAddr = "203.0.113.5:40000".parse().unwrap();
            let peer = self.rs.pm.get_or(PEER_ID).await;
            let mut w = peer.write().await;
            w.socket_addr = peer_addr;
            w.last_reg_time = Instant::now();
            peer_addr
        }

        async fn punch(&mut self) -> (PunchHole, Option<SocketAddr>) {
            let ph = PunchHoleRequest {
                id: PEER_ID.to_owned(),
                licence_key: self.key.clone(),
                nat_type: NatType::ASYMMETRIC.into(),
                ..Default::default()
            };
            let from: SocketAddr = "198.51.100.7:50000".parse().unwrap();
            let key = self.key.clone();
            let (msg, to) = self
                .rs
                .handle_punch_hole_request(from, ph, &key, false)
                .await
                .unwrap();
            match msg.union {
                Some(rendezvous_message::Union::PunchHole(p)) => (p, to),
                other => panic!("expected PunchHole, got {other:?}"),
            }
        }
    }

    #[hbb_common::tokio::test]
    async fn relay_servers_set_via_handle_drive_relay_selection() {
        let mut f = fixture().await;
        assert_eq!(f.relays(), [RELAY_INIT]);
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        assert_eq!(f.rs.get_relay_server(ip, ip), RELAY_INIT);

        f.handle
            .set_relay_servers(vec![RELAY_A.to_owned(), String::new(), RELAY_B.to_owned()])
            .await
            .unwrap();
        f.pump().await;
        assert_eq!(f.relays(), [RELAY_A, RELAY_B], "empty entry dropped");
        f.assert_picks_from_relays();

        let peer_addr = f.online_peer().await;
        let (ph, to) = f.punch().await;
        assert_eq!(to, Some(peer_addr));
        assert!(f.relays().contains(&ph.relay_server), "{}", ph.relay_server);

        f.handle.set_relay_servers(Vec::new()).await.unwrap();
        f.pump().await;
        assert_eq!(f.rs.get_relay_server(ip, ip), "");
    }

    // deskhaven (M1-06): a connection task works on a clone of the server taken at accept;
    // relay changes made by the loop afterwards (handle update, health check) still reach it.
    #[hbb_common::tokio::test]
    async fn connection_clone_sees_later_relay_changes() {
        let mut f = fixture().await;
        let conn = f.rs.clone();
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        assert_eq!(conn.get_relay_server(ip, ip), RELAY_INIT);

        f.handle
            .set_relay_servers(vec![RELAY_A.to_owned()])
            .await
            .unwrap();
        f.pump().await;
        assert_eq!(conn.get_relay_server(ip, ip), RELAY_A, "handle update");

        f.handle
            .set_relay_servers(vec![RELAY_B.to_owned(), RELAY_C.to_owned()])
            .await
            .unwrap();
        f.pump().await;
        let checked = f.rs.relay_servers0.clone();
        f.tx.send(Data::RelayServers(checked, vec![RELAY_C.to_owned()]))
            .unwrap();
        f.pump().await;
        assert_eq!(conn.get_relay_server(ip, ip), RELAY_C, "health check");
    }

    // deskhaven (M1-06): relay selection is the control plane's: it gets both addresses and
    // the current list, and `None` hands out no relay.
    #[hbb_common::tokio::test]
    async fn relay_choice_is_delegated_to_the_control_plane() {
        use crate::control::{Authz, ConnectReq, ControlPlane, RegisterInfo, RegisterVerdict};
        use hbb_common::rendezvous_proto::{HttpProxyRequest, HttpProxyResponse};
        use std::sync::Mutex as StdMutex;

        #[derive(Default)]
        struct Recorder(StdMutex<Vec<(IpAddr, IpAddr, Vec<String>)>>);
        #[async_trait::async_trait]
        impl ControlPlane for Recorder {
            fn register_pk(&self, _: &RegisterInfo<'_>) -> RegisterVerdict {
                RegisterVerdict::Ok
            }
            async fn authorize(&self, _: &ConnectReq<'_>) -> Authz {
                Authz::Deny(String::new())
            }
            async fn http_proxy(&self, _: SocketAddr, _: HttpProxyRequest) -> HttpProxyResponse {
                HttpProxyResponse::default()
            }
            async fn health_check(&self, _: &str, _: SocketAddr) -> Option<i32> {
                None
            }
            fn pick_relay(&self, a: IpAddr, b: IpAddr, relays: &[String]) -> Option<String> {
                let mut calls = self.0.lock().unwrap();
                calls.push((a, b, relays.to_vec()));
                (calls.len() == 1).then(|| "picked:21117".to_owned())
            }
        }

        let mut f = fixture().await;
        let recorder = Arc::new(Recorder::default());
        f.rs.control = recorder.clone();
        let peer_addr = f.online_peer().await;

        let (first, _) = f.punch().await;
        let (second, _) = f.punch().await;

        assert_eq!(first.relay_server, "picked:21117");
        assert_eq!(second.relay_server, "", "None hands out no relay");
        let from: IpAddr = "198.51.100.7".parse().unwrap();
        let expected = (from, peer_addr.ip(), vec![RELAY_INIT.to_owned()]);
        assert_eq!(*recorder.0.lock().unwrap(), [expected.clone(), expected]);
    }

    #[hbb_common::tokio::test]
    async fn always_use_relay_set_via_handle_forces_relay() {
        let mut f = fixture().await;
        f.online_peer().await;
        let (ph, _) = f.punch().await;
        assert_eq!(
            ph.nat_type,
            NatType::ASYMMETRIC.into(),
            "no forced relay by default"
        );

        f.handle.set_always_use_relay(true).unwrap();
        let (ph, _) = f.punch().await;
        assert_eq!(
            ph.nat_type,
            NatType::SYMMETRIC.into(),
            "SYMMETRIC forces relay"
        );

        f.handle.set_always_use_relay(false).unwrap();
        let (ph, _) = f.punch().await;
        assert_eq!(ph.nat_type, NatType::ASYMMETRIC.into());
    }

    // deskhaven (M1-03): a health-check result computed from a list that has since been
    // replaced must not resurrect the old servers.
    #[hbb_common::tokio::test]
    async fn stale_health_check_result_is_ignored() {
        let mut f = fixture().await;
        f.handle
            .set_relay_servers(vec![RELAY_A.to_owned(), RELAY_B.to_owned()])
            .await
            .unwrap();
        f.pump().await;
        let checked = f.rs.relay_servers0.clone(); // a check of [A, B] starts

        f.handle
            .set_relay_servers(vec![RELAY_C.to_owned(), RELAY_D.to_owned()])
            .await
            .unwrap();
        f.tx.send(Data::RelayServers(checked, vec![RELAY_A.to_owned()]))
            .unwrap();
        f.pump().await;
        assert_eq!(f.relays(), [RELAY_C, RELAY_D], "A/B never come back");

        // A result for the current list still applies.
        let current = f.rs.relay_servers0.clone();
        f.tx.send(Data::RelayServers(current, vec![RELAY_D.to_owned()]))
            .unwrap();
        f.pump().await;
        assert_eq!(f.relays(), [RELAY_D]);
    }

    // deskhaven (M1-03): the initial relay list is resolved on the timer, not in `new` (so the
    // listeners are up before any DNS), and entries that do not resolve are retried on later
    // ticks instead of being dropped for good.
    #[hbb_common::tokio::test]
    async fn initial_relays_resolve_on_timer_and_unresolved_entries_are_retried() {
        let mut f = fixture_with(&[RELAY_INIT, UNRESOLVABLE]).await;
        assert!(f.relays().is_empty(), "nothing is resolved in new");

        f.tick_and_resolve().await;
        assert_eq!(f.relays(), [RELAY_INIT]);
        assert_eq!(f.rs.relay_unresolved, Some(vec![UNRESOLVABLE.to_owned()]));

        // A health-check result narrowed the list; an unchanged retry must not undo it.
        f.rs.relay_servers.store(Arc::new(Vec::new()));
        f.tick_and_resolve().await; // the retry runs again
        assert!(f.relays().is_empty(), "health-check result kept");
        assert_eq!(
            f.rs.relay_unresolved,
            Some(vec![UNRESOLVABLE.to_owned()]),
            "still retried"
        );
    }

    // deskhaven (M1-03): a retry of the initial list still in flight when the handle replaces
    // the list must not restore the initial entries.
    #[hbb_common::tokio::test]
    async fn stale_initial_resolution_is_ignored() {
        let mut f = fixture_with(&[RELAY_INIT, UNRESOLVABLE]).await;
        let config = f.rs.relay_config.clone();
        f.handle
            .set_relay_servers(vec![RELAY_A.to_owned()])
            .await
            .unwrap();
        f.tx.send(Data::RelayResolved {
            config,
            accepted: vec![RELAY_INIT.to_owned()],
            rejected: vec![UNRESOLVABLE.to_owned()],
        })
        .unwrap();
        f.pump().await;
        assert_eq!(f.relays(), [RELAY_A]);
        assert_eq!(
            f.rs.relay_unresolved,
            Some(Vec::new()),
            "no retry of the old list"
        );
    }

    // deskhaven (M1-03): an empty `local_ip` means auto-detect, like upstream `get_arg_or`.
    #[hbb_common::tokio::test]
    async fn empty_local_ip_falls_back_to_detected_ip() {
        async fn local_ip(local_ip: Option<&str>) -> String {
            let cfg = crate::SignalConfig {
                mask: Some("192.168.0.0/16".parse().unwrap()),
                local_ip: local_ip.map(str::to_owned),
                ..crate::SignalConfig::new(sign::gen_keypair().1)
            };
            let (tx, _rx) = mpsc::unbounded_channel();
            let tasks = crate::tasks::Tasks::new(Default::default());
            let control = Arc::new(crate::AllowAll::new());
            let rs =
                RendezvousServer::new(&cfg, memory_store(), control, tx, Default::default(), tasks)
                    .await
                    .unwrap();

            rs.inner.local_ip.clone()
        }
        let detected = local_ip_address::local_ip()
            .map(|x| x.to_string())
            .unwrap_or_default();
        assert_eq!(local_ip(Some("")).await, detected);
        assert_eq!(local_ip(None).await, detected);
        assert_eq!(local_ip(Some("10.1.1.1")).await, "10.1.1.1");
    }

    // deskhaven (M1-03): IP_BLOCKER entries are pruned with upstream's `ib` predicate (kept
    // while either the 60 s register counter or the daily id set is still live).
    #[test]
    fn ip_blocks_pruned_once_both_windows_expired() {
        let t0 = Instant::now();
        let entry =
            |counter_at: Instant, ids_at: Instant| ((1, counter_at), (Default::default(), ids_at));
        let day = Duration::from_secs(DAY_SECONDS);
        let later = t0 + day;
        let mut map: IpBlockMap = HashMap::new();
        map.insert("idle".to_owned(), entry(t0, t0));
        map.insert("counter-live".to_owned(), entry(later, t0));
        map.insert("ids-live".to_owned(), entry(t0, later));

        prune_ip_blocks(&mut map, t0 + day);
        assert_eq!(map.len(), 3, "the daily id window is still live for all");

        prune_ip_blocks(&mut map, later + Duration::from_secs(1));
        let mut kept: Vec<_> = map.keys().cloned().collect();
        kept.sort();
        assert_eq!(
            kept,
            ["counter-live", "ids-live"],
            "idle: both windows expired"
        );

        prune_ip_blocks(&mut map, later + day + Duration::from_secs(1));
        assert!(map.is_empty(), "both windows expired for all");
    }

    // deskhaven (M1-03): IP_BLOCKER is capped; when full, new IPs are refused (TOO_FREQUENT)
    // without an insert, while known IPs keep their normal accounting.
    #[test]
    fn ip_block_cap_refuses_new_ips_only() {
        const CAP: usize = 2;
        let mut map: IpBlockMap = HashMap::new();
        assert!(check_ip_block(&mut map, "1.1.1.1", "id1", CAP));
        assert!(check_ip_block(&mut map, "2.2.2.2", "id2", CAP));

        assert!(
            !check_ip_block(&mut map, "3.3.3.3", "id3", CAP),
            "full: new ip refused"
        );
        assert!(!map.contains_key("3.3.3.3"), "and not inserted");

        assert!(
            check_ip_block(&mut map, "1.1.1.1", "id1", CAP),
            "known ip unaffected"
        );
        assert_eq!(map.len(), CAP);

        map.remove("2.2.2.2"); // pruning frees a slot
        assert!(check_ip_block(&mut map, "3.3.3.3", "id3", CAP));
    }
}
