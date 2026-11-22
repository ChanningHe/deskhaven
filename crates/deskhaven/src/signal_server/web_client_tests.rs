//! s5: the core's web client port ([`WebPunches`]) over a real, supervised signal server. A
//! relay-only punch reaches a simulated device as a PunchHole that forces relay; the device's
//! RelayResponse comes back as the session's uuid and the device key the server signed, which
//! the server's public key verifies.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use base64_022::Engine as _;
use base64_022::engine::general_purpose::STANDARD;
use deskhaven_core::web_client::{PunchError, PunchRequest, Punched, WebPunches};
use hbb_common::bytes_codec::BytesCodec;
use hbb_common::futures_util::{SinkExt as _, StreamExt as _};
use hbb_common::protobuf::Message as _;
use hbb_common::rendezvous_proto::{
    IdPk, NatType, PunchHole, RelayResponse, RendezvousMessage, rendezvous_message,
};
use hbb_common::tokio_util::codec::Framed;
use serde_json::json;
use sodiumoxide::crypto::sign;
use tokio::net::TcpSocket;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep, timeout};
use tokio_util::sync::CancellationToken;

use super::tests::{POLL, STEP, TARGET, TARGET_IP, Target, memory_store, start_launcher, with};

/// TEST-NET-1: never probed healthy, never replaced.
const RELAY: &str = "192.0.2.1:21117";
/// The browser's IP as the HTTP stack resolved it.
const BROWSER_IP: IpAddr = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7));
const RELAY_UUID: &str = "e4c181d0-41c7-4ae6-8ef0-36581b6b2b12";
/// Any non-empty client version makes the server sign the device key into its answer.
const CLIENT_VERSION: &str = "1.4.0";
/// The key `Target::register` registers.
const TARGET_PK: [u8; 32] = [9; 32];

/// The device's answer to `ph` (`create_relay`), sent the way a device does: on a new TCP
/// connection from its own address, after the server's KeyExchange offer.
async fn answer_from_device(port: u16, ph: &PunchHole) {
    let socket = TcpSocket::new_v4().unwrap();
    socket.bind(SocketAddr::new(TARGET_IP, 0)).unwrap();
    let stream = socket
        .connect(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))
        .await
        .unwrap();
    let mut conn = Framed::new(stream, BytesCodec::new());
    let offer = timeout(STEP, conn.next()).await.unwrap().unwrap().unwrap();
    let offer = RendezvousMessage::parse_from_bytes(&offer).unwrap();
    assert!(matches!(
        offer.union,
        Some(rendezvous_message::Union::KeyExchange(_))
    ));
    let mut rr = RelayResponse {
        socket_addr: ph.socket_addr.clone(),
        uuid: RELAY_UUID.to_owned(),
        relay_server: RELAY.to_owned(),
        version: CLIENT_VERSION.to_owned(),
        ..Default::default()
    };
    rr.set_id(TARGET.to_owned());
    let mut msg = RendezvousMessage::new();
    msg.set_relay_response(rr);
    conn.send(msg.write_to_bytes().unwrap().into())
        .await
        .unwrap();
}

/// A punch of [`TARGET`] in flight, started once the relay settings reached the server
/// (they apply asynchronously after it starts; before, a punch fails at once with
/// `NoRelay`).
async fn punch_once_relays_apply(port: &Arc<dyn WebPunches>) -> JoinHandle<PunchResult> {
    let deadline = Instant::now() + STEP;
    loop {
        let punch = tokio::spawn({
            let port = Arc::clone(port);
            async move { port.punch(request(TARGET)).await }
        });
        sleep(POLL).await;
        if !punch.is_finished() {
            return punch;
        }
        let res = punch.await.unwrap();
        assert!(
            matches!(res, Err(PunchError::NoRelay)),
            "{:?}",
            res.map(|_| ())
        );
        assert!(Instant::now() < deadline, "the relay setting never applied");
    }
}

type PunchResult = Result<Punched, PunchError>;

fn request(device_id: &str) -> PunchRequest {
    PunchRequest {
        device_id: device_id.to_owned(),
        offer: None,
        client: BROWSER_IP,
    }
}

#[tokio::test]
async fn a_web_punch_through_the_port_returns_a_relay_session_with_a_signed_device_key() {
    // Arrange
    let settings = with(&[("signal.relay_servers", json!([RELAY]))]);
    let (_tx, rx) = watch::channel(Arc::new(settings));
    let token = CancellationToken::new();
    let (running, port, key) = start_launcher(rx, memory_store(), &token).await;
    let server_pk = sign::PublicKey::from_slice(&STANDARD.decode(key).unwrap()).unwrap();
    let mut target = Target::register(port).await;
    let port_impl: Arc<dyn WebPunches> = Arc::new(running.status.clone());

    // Act
    let punch = punch_once_relays_apply(&port_impl).await;
    let ph = match target.recv().await.union {
        Some(rendezvous_message::Union::PunchHole(ph)) => ph,
        other => panic!("expected PunchHole, got {other:?}"),
    };
    answer_from_device(port, &ph).await;
    let punched = timeout(STEP, punch).await.unwrap().unwrap().unwrap();
    let unknown = port_impl.punch(request("987654321")).await;

    // Assert
    assert_eq!(ph.relay_server, RELAY);
    assert_eq!(ph.nat_type.enum_value(), Ok(NatType::SYMMETRIC));
    assert_eq!(ph.webrtc_sdp_offer, "");
    assert_eq!(punched.uuid, RELAY_UUID);
    assert_eq!(punched.relay_server, RELAY);
    assert_eq!(punched.answer, None);
    let signed = sign::verify(&punched.signed_pk, &server_pk).expect("signed by the server");
    let id_pk = IdPk::parse_from_bytes(&signed).unwrap();
    assert_eq!(id_pk.id, TARGET);
    assert_eq!(id_pk.pk.as_ref(), TARGET_PK);
    let mut receiver = punched.receiver;
    let no_webrtc = timeout(STEP, receiver.next()).await.unwrap();
    assert_eq!(no_webrtc, None, "a relay-only route carries no candidates");
    assert_eq!(unknown.map(|_| ()), Err(PunchError::NotFound));
    token.cancel();
    timeout(STEP, running.task).await.unwrap().unwrap();
}
