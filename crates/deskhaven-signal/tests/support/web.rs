use super::{
    CLIENT_VERSION, Conn, KxClient, QUIET, Recv, TcpClient, TestServer, UdpClient, msg::rv,
    read_offer, secure,
};
use deskhaven_signal::{SignalConfig, WebPunch, WebPunchError, WebRoute};
use hbb_common::{
    bytes::Bytes,
    rendezvous_proto::{IceCandidate, PunchHole, RelayResponse, RendezvousMessage},
    tokio::{self, task::JoinHandle},
};
use std::net::{IpAddr, Ipv4Addr};

/// The deployment's only relay (TEST-NET-1: never probed healthy, never replaced).
pub const WEB_RELAY: &str = "192.0.2.80:21117";
pub const FOREIGN_RELAY: &str = "203.0.113.9:21117";
pub const BROWSER_IP: IpAddr = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7));
pub const FINGERPRINT: &str = "sha-256 5E:8E:4F:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:\
                               00:11:22:33:44:55:66:77:88:99:AA:BB:CC";
pub const ANSWER: &str = "webrtc://eyJ0eXBlIjoiYW5zd2VyIn0=";
pub const RELAY_UUID: &str = "e4c181d0-41c7-4ae6-8ef0-36581b6b2b12";

/// The fingerprint sits at session level, as Chromium puts it.
pub fn offer() -> String {
    let sdp = format!(
        "v=0\r\no=- 3905815367653238 2 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n\
         a=group:BUNDLE 0\r\na=fingerprint:{FINGERPRINT}\r\n\
         m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\nc=IN IP4 0.0.0.0\r\n\
         a=setup:actpass\r\na=mid:0\r\na=sctp-port:5000\r\n"
    );
    let json = hbb_common::serde_json::json!({ "type": "offer", "sdp": sdp }).to_string();
    format!("webrtc://{}", base64::encode(json))
}

/// The handle's update and the punch travel the server loop's queue in order.
pub async fn web_server(customize: impl Fn(SignalConfig) -> SignalConfig) -> TestServer {
    let server = TestServer::start_with(customize).await;
    server
        .handle
        .set_relay_servers(vec![WEB_RELAY.to_owned()])
        .await
        .unwrap();
    server
}

pub fn punch(
    server: &TestServer,
    target: &str,
    offer: Option<String>,
) -> JoinHandle<Result<WebRoute, WebPunchError>> {
    let handle = server.handle.clone();
    let req = WebPunch {
        target: target.to_owned(),
        offer,
        client: BROWSER_IP,
    };
    tokio::spawn(async move { handle.web_punch(req).await })
}

pub fn relay_answer(ph: &PunchHole, target: &str, relay: &str, answer: &str) -> RelayResponse {
    let mut rr = RelayResponse {
        socket_addr: ph.socket_addr.clone(),
        uuid: RELAY_UUID.to_owned(),
        relay_server: relay.to_owned(),
        version: CLIENT_VERSION.to_owned(),
        webrtc_sdp_answer: answer.to_owned(),
        ..Default::default()
    };
    rr.set_id(target.to_owned());
    rr
}

/// The server's KeyExchange offer is read first, so closing afterwards sends a FIN behind the
/// frame (closing with unread data would reset it).
pub async fn send_relay_response(server: &TestServer, ip: IpAddr, rr: RelayResponse) {
    let mut conn = TcpClient::connect_from(ip, server.main_addr()).await;
    read_offer(&mut conn).await;
    conn.send(&rv(|m| m.set_relay_response(rr))).await;
}

pub async fn candidate_conn(server: &TestServer, ip: IpAddr) -> TcpClient {
    let mut conn = TcpClient::connect_from(ip, server.main_addr()).await;
    secure(&mut conn, &server.public_key, KxClient::Current)
        .await
        .unwrap();
    conn
}

pub fn peer_candidate(ctl: &Bytes, session_key: &str, candidate: &str) -> RendezvousMessage {
    rv(|m| {
        m.set_ice_candidate(IceCandidate {
            socket_addr: ctl.clone(),
            session_key: session_key.to_owned(),
            candidate: candidate.to_owned(),
            ..Default::default()
        })
    })
}

pub async fn answered_punch<C: Conn>(
    server: &TestServer,
    peer: &mut C,
    ip: IpAddr,
    target: &str,
    offer: Option<String>,
    relay: &str,
    answer: &str,
) -> (PunchHole, Result<WebRoute, WebPunchError>) {
    let task = punch(server, target, offer);
    let ph = expect_msg!(peer.recv().await, PunchHole);
    send_relay_response(server, ip, relay_answer(&ph, target, relay, answer)).await;
    (ph, task.await.unwrap())
}

pub const MAX_ICE_MISSES: usize = 16;

pub async fn webrtc_route(server: &TestServer, target: &str) -> (UdpClient, Bytes, WebRoute) {
    let (mut peer, _) = server.online_peer(target).await;
    let ip = peer.local_addr().ip();
    let offer = Some(offer());
    let (ph, res) = answered_punch(server, &mut peer, ip, target, offer, WEB_RELAY, ANSWER).await;
    (peer, ph.socket_addr, res.unwrap())
}

pub async fn assert_route_gone(server: &TestServer, ip: IpAddr, ctl: &Bytes) {
    let mut ice = candidate_conn(server, ip).await;
    for i in 0..MAX_ICE_MISSES {
        ice.send(&peer_candidate(ctl, FINGERPRINT, &format!("gone-{i}")))
            .await;
    }
    assert!(matches!(ice.recv_within(QUIET).await, Recv::Quiet));
    ice.send(&peer_candidate(ctl, FINGERPRINT, "gone-last"))
        .await;
    ice.assert_closed().await;
}
