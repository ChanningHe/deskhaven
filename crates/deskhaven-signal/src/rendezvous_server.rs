// Modified by deskhaven; see crates/deskhaven-signal/CHANGES.md (AGPL-3.0 §5a).
use crate::common::*;
use crate::peer::*;
use crate::server::{Ports, SignalConfig, SignalError};
use crate::tasks::Tasks;
use hbb_common::{
    AddrMangle, ResultType, allow_err, bail,
    bytes::{Bytes, BytesMut},
    bytes_codec::BytesCodec,
    config,
    futures::future::join_all,
    futures_util::{
        sink::SinkExt,
        stream::{SplitSink, StreamExt},
    },
    log,
    protobuf::{Message as _, MessageField},
    rendezvous_proto::{
        register_pk_response::Result::{TOO_FREQUENT, UUID_MISMATCH},
        *,
    },
    tcp::FramedStream,
    timeout,
    tokio::{
        self,
        net::{TcpListener, TcpStream},
        sync::{Mutex, mpsc},
        time::{Duration, interval},
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
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Instant,
};

#[derive(Clone, Debug)]
pub(crate) enum Data {
    Msg(Box<RendezvousMessage>, SocketAddr),
    // deskhaven: a list (from `SignalHandle::set_relay_servers`) instead of the console's
    // comma-separated string.
    RelayServers0(RelayServers),
    RelayServers(RelayServers),
}

const REG_TIMEOUT: i64 = 30_000;
type TcpStreamSink = SplitSink<Framed<TcpStream, BytesCodec>, Bytes>;
type WsSink = SplitSink<tokio_tungstenite::WebSocketStream<TcpStream>, tungstenite::Message>;
enum Sink {
    TcpStream(TcpStreamSink),
    Ws(WsSink),
}
pub(crate) type Sender = mpsc::UnboundedSender<Data>;
pub(crate) type Receiver = mpsc::UnboundedReceiver<Data>;
static ROTATION_RELAY_SERVER: AtomicUsize = AtomicUsize::new(0);
type RelayServers = Vec<String>;
const CHECK_RELAY_TIMEOUT: u64 = 3_000;
// deskhaven: the ALWAYS_USE_RELAY static became the per-instance `always_use_relay` field.

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
    serial: i32,
    // deskhaven: `version` / `software_url` (deprecated `-u`, SoftwareUpdate answers) removed.
    mask: Option<Ipv4Network>,
    local_ip: String,
    // deskhaven: always present, injected through `SignalConfig::secret_key`.
    sk: sign::SecretKey,
    punch_req_max: usize,
}

#[derive(Clone)]
pub(crate) struct RendezvousServer {
    tcp_punch: Arc<Mutex<HashMap<SocketAddr, Sink>>>,
    pm: PeerMap,
    tx: Sender,
    relay_servers: Arc<RelayServers>,
    relay_servers0: Arc<RelayServers>,
    rendezvous_servers: Arc<Vec<String>>,
    inner: Arc<Inner>,
    always_use_relay: Arc<AtomicBool>,
    tasks: Tasks,
}

enum LoopFailure {
    UdpSocket,
    Listener3,
    Listener2,
    Listener,
}

impl RendezvousServer {
    // deskhaven: replaces upstream `start` / `start_with_bind` (own tokio runtime, env-var
    // arguments, own SIGTERM/INT/QUIT handling, loopback console, exiting the process). Runs on
    // the caller's runtime until a listener cannot be re-created or the UDP self-test fails;
    // the caller (`SignalServer::run`) handles cancellation.
    pub(crate) async fn serve(
        cfg: SignalConfig,
        ports: Ports,
        tx: Sender,
        mut rx: Receiver,
        always_use_relay: Arc<AtomicBool>,
        tasks: Tasks,
    ) -> Result<(), SignalError> {
        let key = cfg.licence_key();
        log::info!("Key: {}", key);
        let bind_addr = cfg.bind;
        let rmem = cfg.rmem;
        let mut rs = Self::new(&cfg, tx, always_use_relay, tasks)
            .await
            .map_err(SignalError::internal)?;
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
    // rendezvous-servers start at 0 / empty (a loopback ConfigureUpdate can still set them).
    async fn new(
        cfg: &SignalConfig,
        tx: Sender,
        always_use_relay: Arc<AtomicBool>,
        tasks: Tasks,
    ) -> ResultType<Self> {
        let pm = PeerMap::new().await?;
        let mask = cfg.mask;
        let local_ip = match (&mask, &cfg.local_ip) {
            (None, _) => "".to_owned(),
            (Some(_), Some(local_ip)) => local_ip.clone(),
            (Some(_), None) => local_ip_address::local_ip()
                .map(|x| x.to_string())
                .unwrap_or_default(),
        };
        let mut rs = Self {
            tcp_punch: Arc::new(Mutex::new(HashMap::new())),
            pm,
            tx,
            relay_servers: Default::default(),
            relay_servers0: Default::default(),
            rendezvous_servers: Default::default(),
            inner: Arc::new(Inner {
                serial: 0,
                sk: cfg.secret_key.clone(),
                mask,
                local_ip,
                punch_req_max: cfg.punch_req_max,
            }),
            always_use_relay,
            tasks,
        };
        log::info!("mask: {:?}", rs.inner.mask);
        log::info!("local-ip: {:?}", rs.inner.local_ip);
        rs.parse_relay_servers(&cfg.relay_servers);
        Ok(rs)
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
                    if self.relay_servers0.len() > 1 {
                        let rs = self.relay_servers0.clone();
                        let tx = self.tx.clone();
                        self.tasks.spawn(async move {
                            check_relay_servers(rs, tx).await;
                        });
                    }
                }
                Some(data) = rx.recv() => {
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
                self.parse_relay_servers(&rs);
            }
            Data::RelayServers(rs) => {
                self.relay_servers = Arc::new(rs);
            }
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
                        if self.inner.serial > rp.serial {
                            let mut msg_out = RendezvousMessage::new();
                            msg_out.set_configure_update(ConfigUpdate {
                                serial: self.inner.serial,
                                rendezvous_servers: (*self.rendezvous_servers).clone(),
                                ..Default::default()
                            });
                            socket.send(&msg_out, addr).await?;
                        }
                    }
                }
                Some(rendezvous_message::Union::RegisterPk(rk)) => {
                    if rk.uuid.is_empty() || rk.pk.is_empty() {
                        return Ok(());
                    }
                    let id = rk.id;
                    let ip = addr.ip().to_string();
                    if id.len() < 6 || id.len() > MAX_PEER_ID_LEN {
                        return send_rk_res(socket, addr, UUID_MISMATCH).await;
                    } else if !self.check_ip_blocker(&ip, &id).await {
                        return send_rk_res(socket, addr, TOO_FREQUENT).await;
                    }
                    let peer = self.pm.get_or(&id).await;
                    let (changed, ip_changed) = {
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
                                    return send_rk_res(socket, addr, UUID_MISMATCH).await;
                                }
                            } else {
                                log::warn!(
                                    "Peer {} uuid mismatch: {:?} vs {:?}",
                                    id,
                                    rk.uuid,
                                    peer.uuid
                                );
                                drop(peer);
                                return send_rk_res(socket, addr, UUID_MISMATCH).await;
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
                        return send_rk_res(socket, addr, TOO_FREQUENT).await;
                    }
                    req_pk.0 += 1;
                    req_pk.1 = Instant::now();
                    peer.write().await.reg_pk = req_pk;
                    if ip_changed {
                        let mut lock = IP_CHANGES.lock().await;
                        if let Some((tm, ips)) = lock.get_mut(&id) {
                            if tm.elapsed().as_secs() > IP_CHANGE_DUR {
                                *tm = Instant::now();
                                ips.clear();
                                ips.insert(ip.clone(), 1);
                            } else if let Some(v) = ips.get_mut(&ip) {
                                *v += 1;
                            } else {
                                ips.insert(ip.clone(), 1);
                            }
                        } else {
                            lock.insert(
                                id.clone(),
                                (Instant::now(), HashMap::from([(ip.clone(), 1)])),
                            );
                        }
                    }
                    if changed {
                        self.pm.update_pk(id, peer, addr, rk.uuid, rk.pk, ip).await;
                    }
                    let mut msg_out = RendezvousMessage::new();
                    msg_out.set_register_pk_response(RegisterPkResponse {
                        result: register_pk_response::Result::OK.into(),
                        ..Default::default()
                    });
                    socket.send(&msg_out, addr).await?
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
                Some(rendezvous_message::Union::ConfigureUpdate(mut cu)) => {
                    if try_into_v4(addr).ip().is_loopback() && cu.serial > self.inner.serial {
                        let mut inner: Inner = (*self.inner).clone();
                        inner.serial = cu.serial;
                        self.inner = Arc::new(inner);
                        self.rendezvous_servers = Arc::new(
                            cu.rendezvous_servers
                                .drain(..)
                                .filter(|x| {
                                    !x.is_empty()
                                        && test_if_valid_server(x, "rendezvous-server").is_ok()
                                })
                                .collect(),
                        );
                        log::info!(
                            "configure updated: serial={} rendezvous-servers={:?}",
                            self.inner.serial,
                            self.rendezvous_servers
                        );
                    }
                }
                // deskhaven: SoftwareUpdate arm removed with the deprecated `-u` software-url
                // option (it only answered when that option was set).
                _ => {}
            }
        }
        Ok(())
    }

    #[inline]
    async fn handle_tcp(
        &mut self,
        bytes: &[u8],
        sink: &mut Option<Sink>,
        addr: SocketAddr,
        key: &str,
        ws: bool,
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
                    // there maybe several attempt, so sink can be none
                    if let Some(sink) = sink.take() {
                        self.tcp_punch.lock().await.insert(try_into_v4(addr), sink);
                    }
                    if let Some(peer) = self.pm.get_in_memory(&rf.id).await {
                        let mut msg_out = RendezvousMessage::new();
                        rf.socket_addr = AddrMangle::encode(addr).into();
                        msg_out.set_request_relay(rf);
                        let peer_addr = peer.read().await.socket_addr;
                        self.tx.send(Data::Msg(msg_out.into(), peer_addr)).ok();
                    }
                    return true;
                }
                Some(rendezvous_message::Union::RelayResponse(mut rr)) => {
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
                Some(rendezvous_message::Union::PunchHoleSent(phs)) => {
                    allow_err!(self.handle_hole_sent(phs, addr, None).await);
                }
                Some(rendezvous_message::Union::LocalAddr(la)) => {
                    allow_err!(self.handle_local_addr(la, addr, None).await);
                }
                Some(rendezvous_message::Union::TestNatRequest(tar)) => {
                    let mut msg_out = RendezvousMessage::new();
                    let mut res = TestNatResponse {
                        port: addr.port() as _,
                        ..Default::default()
                    };
                    if self.inner.serial > tar.serial {
                        let mut cu = ConfigUpdate::new();
                        cu.serial = self.inner.serial;
                        cu.rendezvous_servers = (*self.rendezvous_servers).clone();
                        res.cu = MessageField::from_option(Some(cu));
                    }
                    msg_out.set_test_nat_response(res);
                    Self::send_to_sink(sink, msg_out).await;
                }
                Some(rendezvous_message::Union::RegisterPk(_)) => {
                    let res = register_pk_response::Result::NOT_SUPPORT;
                    let mut msg_out = RendezvousMessage::new();
                    msg_out.set_register_pk_response(RegisterPkResponse {
                        result: res.into(),
                        ..Default::default()
                    });
                    Self::send_to_sink(sink, msg_out).await;
                }
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
        let mut msg_out = RendezvousMessage::new();
        msg_out.set_register_peer_response(RegisterPeerResponse {
            request_pk,
            ..Default::default()
        });
        socket.send(&msg_out, socket_addr).await
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
        if let Some(sink) = sink.as_mut()
            && let Ok(bytes) = msg.write_to_bytes()
        {
            match sink {
                Sink::TcpStream(s) => {
                    allow_err!(s.send(Bytes::from(bytes)).await);
                }
                Sink::Ws(ws) => {
                    allow_err!(ws.send(tungstenite::Message::Binary(bytes.into())).await);
                }
            }
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
        let (msg, to_addr) = self.handle_punch_hole_request(addr, ph, key, ws).await?;
        if let Some(addr) = to_addr {
            self.tx.send(Data::Msg(msg.into(), addr))?;
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

    async fn check_ip_blocker(&self, ip: &str, id: &str) -> bool {
        let mut lock = IP_BLOCKER.lock().await;
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
        } else {
            lock.insert(ip.to_owned(), ((0, now), (Default::default(), now)));
        }
        true
    }

    // deskhaven: takes a list (config / `SignalHandle`) instead of a comma-separated string.
    // The loopback console (`check_cmd`) that followed here was removed: unauthenticated, and
    // its `aur` command overwrote the relay list with "Y"/"N".
    fn parse_relay_servers(&mut self, relay_servers: &[String]) {
        let rs = get_servers(relay_servers, "relay-servers");
        self.relay_servers0 = Arc::new(rs);
        self.relay_servers = self.relay_servers0.clone();
    }

    fn get_relay_server(&self, _pa: IpAddr, _pb: IpAddr) -> String {
        if self.relay_servers.is_empty() {
            return "".to_owned();
        } else if self.relay_servers.len() == 1 {
            return self.relay_servers[0].clone();
        }
        let i = ROTATION_RELAY_SERVER.fetch_add(1, Ordering::SeqCst) % self.relay_servers.len();
        self.relay_servers[i].clone()
    }

    async fn handle_listener2(&self, stream: TcpStream, addr: SocketAddr) {
        let mut rs = self.clone();
        // deskhaven: loopback connections are no longer diverted to the removed console; they
        // get the same NAT test / online query handling as everyone else.
        let stream = FramedStream::from(stream, addr);
        self.tasks.spawn(async move {
            let mut stream = stream;
            if let Some(Ok(bytes)) = stream.next_timeout(30_000).await
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
        let mut sink;
        if ws {
            use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
            let callback = |req: &Request, response: Response| {
                let headers = req.headers();
                // X-Real-IP / X-Forwarded-For are trusted as-is so that the real
                // client IP is preserved when the WebSocket port runs behind a
                // reverse proxy (WSS). They are NOT validated: anyone who can reach
                // this port directly can spoof an arbitrary IP, bypassing IP-based
                // rate limiting / blocking and corrupting logged IPs. Do not expose
                // the WebSocket port directly to untrusted networks; only the
                // reverse proxy, which overwrites these headers, should be able to
                // connect to it.
                // https://github.com/rustdesk/rustdesk-server/issues/634
                let real_ip = headers
                    .get("X-Real-IP")
                    .or_else(|| headers.get("X-Forwarded-For"))
                    .and_then(|header_value| header_value.to_str().ok());
                if let Some(ip) = real_ip {
                    if ip.contains('.') {
                        addr = format!("{ip}:0").parse().unwrap_or(addr);
                    } else {
                        addr = format!("[{ip}]:0").parse().unwrap_or(addr);
                    }
                }
                Ok(response)
            };
            let ws_stream = tokio_tungstenite::accept_hdr_async(stream, callback).await?;
            let (a, mut b) = ws_stream.split();
            sink = Some(Sink::Ws(a));
            while let Ok(Some(Ok(msg))) = timeout(30_000, b.next()).await {
                if let tungstenite::Message::Binary(bytes) = msg
                    && !self.handle_tcp(&bytes, &mut sink, addr, key, ws).await
                {
                    break;
                }
            }
        } else {
            let (a, mut b) = Framed::new(stream, BytesCodec::new()).split();
            sink = Some(Sink::TcpStream(a));
            while let Ok(Some(Ok(bytes))) = timeout(30_000, b.next()).await {
                if !self.handle_tcp(&bytes, &mut sink, addr, key, ws).await {
                    break;
                }
            }
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
        tx.send(Data::RelayServers(rs)).ok();
    }
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

#[inline]
async fn send_rk_res(
    socket: &mut FramedSocket,
    addr: SocketAddr,
    res: register_pk_response::Result,
) -> ResultType<()> {
    let mut msg_out = RendezvousMessage::new();
    msg_out.set_register_pk_response(RegisterPkResponse {
        result: res.into(),
        ..Default::default()
    });
    socket.send(&msg_out, addr).await
}

async fn create_udp_listener(
    bind_addr: Option<IpAddr>,
    port: i32,
    rmem: usize,
) -> ResultType<FramedSocket> {
    if let Some(bind_addr) = bind_addr {
        let addr = SocketAddr::new(bind_addr, port as _);
        return FramedSocket::new_reuse(&addr, true, rmem).await;
    }
    let addr = SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port as _);
    if let Ok(s) = FramedSocket::new_reuse(&addr, true, rmem).await {
        log::debug!("listen on udp {:?}", s.local_addr());
        return Ok(s);
    }
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port as _);
    let s = FramedSocket::new_reuse(&addr, true, rmem).await?;
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
    const PEER_ID: &str = "relay-test-peer";
    const RELAY_A: &str = "10.0.0.1:21117";
    const RELAY_B: &str = "10.0.0.2:21117";

    struct Fixture {
        rs: RendezvousServer,
        handle: crate::SignalHandle,
        rx: Receiver,
        socket: FramedSocket,
        key: String,
    }

    async fn fixture() -> Fixture {
        let cfg = crate::SignalConfig {
            relay_servers: vec!["10.9.9.9:21117".to_owned()],
            punch_req_max: 0, // keep the process-wide PUNCH_REQS untouched
            ..crate::SignalConfig::new(sign::gen_keypair().1)
        };
        let (tx, rx) = mpsc::unbounded_channel();
        let always_use_relay = Arc::new(AtomicBool::new(cfg.always_use_relay));
        let handle = crate::SignalHandle::new(tx.clone(), always_use_relay.clone());
        let tasks = crate::tasks::Tasks::new(Default::default());
        let rs = RendezvousServer::new(&cfg, tx, always_use_relay, tasks)
            .await
            .unwrap();
        let socket = create_udp_listener(Some(IpAddr::V4(Ipv4Addr::LOCALHOST)), 0, 0)
            .await
            .unwrap();
        let key = cfg.licence_key();
        Fixture {
            rs,
            handle,
            rx,
            socket,
            key,
        }
    }

    impl Fixture {
        /// Delivers queued handle updates the way `io_loop` does.
        async fn pump(&mut self) {
            while let Ok(data) = self.rx.try_recv() {
                self.rs.handle_data(data, &mut self.socket).await;
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
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        assert_eq!(f.rs.get_relay_server(ip, ip), "10.9.9.9:21117");

        f.handle
            .set_relay_servers(vec![RELAY_A.to_owned(), String::new(), RELAY_B.to_owned()])
            .unwrap();
        f.pump().await;

        let picks: std::collections::HashSet<String> =
            (0..4).map(|_| f.rs.get_relay_server(ip, ip)).collect();
        let expected = [RELAY_A.to_owned(), RELAY_B.to_owned()]
            .into_iter()
            .collect();
        assert_eq!(
            picks, expected,
            "round-robin over the new list, empty entry dropped"
        );

        let peer_addr = f.online_peer().await;
        let (ph, to) = f.punch().await;
        assert_eq!(to, Some(peer_addr));
        assert!(expected.contains(&ph.relay_server), "{}", ph.relay_server);

        f.handle.set_relay_servers(Vec::new()).unwrap();
        f.pump().await;
        assert_eq!(f.rs.get_relay_server(ip, ip), "");
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

        f.handle.set_always_use_relay(true);
        let (ph, _) = f.punch().await;
        assert_eq!(
            ph.nat_type,
            NatType::SYMMETRIC.into(),
            "SYMMETRIC forces relay"
        );

        f.handle.set_always_use_relay(false);
        let (ph, _) = f.punch().await;
        assert_eq!(ph.nat_type, NatType::ASYMMETRIC.into());
    }
}
