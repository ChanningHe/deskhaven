//! End-to-end tests of the signal server inside `deskhaven serve` (M1-06): the real binary
//! on free ports, RustDesk wire protocol (hbb_common protobufs) over UDP and TCP.
//! Requires Linux routing all of 127.0.0.0/8 to `lo` (the controlled peer uses 127.0.0.2).
#![cfg(unix)]
// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]

mod support;

use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use deskhaven_core::devices::DeviceRepository;
use hbb_common::bytes_codec::BytesCodec;
use hbb_common::futures_util::{SinkExt as _, StreamExt as _};
use hbb_common::protobuf::Message as _;
use hbb_common::rendezvous_proto::{
    IdPk, NatType, PunchHoleRequest, RegisterPeer, RegisterPk, RelayResponse, RendezvousMessage,
    RequestRelay, TestNatRequest, punch_hole_response, register_pk_response, rendezvous_message,
};
use hbb_common::tokio_util::codec::Framed;
use sodiumoxide::crypto::sign;
use support::*;

const TARGET: &str = "123456789";
const TARGET_IP: IpAddr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));
const LOCALHOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
/// TEST-NET-1 (RFC 5737): resolves without DNS, never passes a health probe.
const PINNED_RELAY: &str = "192.0.2.1:21117";
const NET_TIMEOUT: Duration = Duration::from_secs(5);
const CLIENT_VERSION: &str = "1.4.2";

fn signal_config(dir: &Path, port: u16, settings: &str) -> String {
    let base = config_with_signal(dir, "", &format!("port = {port}"));
    format!("{base}\n[settings]\n{settings}\n")
}

fn key_show(dir: &Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_deskhaven"))
        .args(["--config", "deskhaven.toml", "key", "show"])
        .env_clear()
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_owned()
}

/// The signal ports answer (TCP NAT test on port-1, UDP RegisterPeer); RegisterPk persists to
/// `devices` and binds the id to its uuid across a restart; the key the server checks licence
/// keys against and signs peer keys with is `key show`'s; a pinned relay list reaches punch
/// holes; `/readyz` reports ready with the signal server serving; SIGTERM stops it cleanly.
#[test]
fn signal_server_serves_persists_and_uses_the_key_show_key() {
    // Arrange
    let dir = workdir("signal-e2e");
    let port = free_signal_port();
    let config = signal_config(
        &dir,
        port,
        &format!("\"signal.relay_servers\" = [\"{PINNED_RELAY}\"]"),
    );
    let mut server = start(&dir, &config, &[]);
    let addr = server.listening("client_api");
    wait_ready(addr);
    let licence_key = key_show(&dir);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let pk = vec![7; 32];

    rt.block_on(async {
        // Act + Assert: NAT test port and UDP registration.
        let nat = request(
            port - 1,
            &rv(|m| m.set_test_nat_request(TestNatRequest::default())),
        )
        .await;
        assert!(
            matches!(
                nat.union,
                Some(rendezvous_message::Union::TestNatResponse(_))
            ),
            "{nat:?}"
        );
        let target = UdpSocket::bind((TARGET_IP, 0)).unwrap();
        target.set_read_timeout(Some(NET_TIMEOUT)).unwrap();
        target.connect((LOCALHOST, port)).unwrap();
        let peer = udp(
            &target,
            &rv(|m| {
                m.set_register_peer(RegisterPeer {
                    id: TARGET.to_owned(),
                    ..Default::default()
                })
            }),
        );
        match peer.union {
            Some(rendezvous_message::Union::RegisterPeerResponse(r)) => assert!(r.request_pk),
            other => panic!("expected RegisterPeerResponse, got {other:?}"),
        }
        assert_eq!(
            register_pk(&target, b"owner-uuid", &pk),
            register_pk_response::Result::OK
        );

        // Licence key = `key show`: accepted, and the pinned relay is handed out.
        let mut controller = Tcp::connect(port).await;
        controller.send(&punch_request(&licence_key)).await;
        match udp_recv(&target).union {
            Some(rendezvous_message::Union::PunchHole(ph)) => {
                assert_eq!(ph.relay_server, PINNED_RELAY);
            }
            other => panic!("expected PunchHole, got {other:?}"),
        }
        let mut stranger = Tcp::connect(port).await;
        stranger.send(&punch_request("not-the-key")).await;
        match stranger.recv().await.union {
            Some(rendezvous_message::Union::PunchHoleResponse(res)) => assert_eq!(
                res.failure.enum_value(),
                Ok(punch_hole_response::Failure::LICENSE_MISMATCH)
            ),
            other => panic!("expected PunchHoleResponse, got {other:?}"),
        }

        // Peer keys are signed with the private half of `key show`'s key.
        let signed = relayed_pk(port, &licence_key, &target).await;
        let public = sign::PublicKey::from_slice(&base64_decode(&licence_key)).unwrap();
        let id_pk = IdPk::parse_from_bytes(&sign::verify(&signed, &public).unwrap()).unwrap();
        assert_eq!(
            (id_pk.id.as_str(), id_pk.pk.as_ref()),
            (TARGET, pk.as_slice())
        );
    });
    let (status, _) = server.terminate();
    let logs = server.logs();
    assert_eq!(status.code(), Some(0), "{logs}");
    assert!(logs.contains("signal server serving"), "{logs}");
    assert_no_hbbs_secret(&dir, &logs);
    let stored = rt.block_on(async {
        let url = format!(
            "sqlite://{}?mode=ro",
            dir.join("data/deskhaven.db").display()
        );
        let db = sea_orm::Database::connect(url).await.unwrap();
        DeviceRepository::new(db)
            .find_by_rustdesk_id(TARGET)
            .await
            .unwrap()
    });
    let stored = stored.expect("devices row");
    assert_eq!(
        (stored.uuid.as_slice(), stored.pk.as_slice()),
        (&b"owner-uuid"[..], pk.as_slice())
    );

    // Act: same data_dir, another device claims the id.
    let mut again = start(&dir, &config, &[]);
    wait_ready(again.listening("client_api"));
    let thief = UdpSocket::bind((IpAddr::V4(Ipv4Addr::new(127, 0, 0, 3)), 0)).unwrap();
    thief.set_read_timeout(Some(NET_TIMEOUT)).unwrap();
    thief.connect((LOCALHOST, port)).unwrap();
    let res = register_pk(&thief, b"thief-uuid", &[8; 32]);

    // Assert
    assert_eq!(res, register_pk_response::Result::UUID_MISMATCH);
    let (status, _) = again.terminate();
    assert_eq!(status.code(), Some(0), "{}", again.logs());
}

/// Startup fails, naming the port, when a signal port is taken (here the WebSocket port).
#[test]
fn taken_signal_port_fails_startup_with_a_clear_error() {
    // Arrange
    let dir = workdir("signal-port-taken");
    let (port, _taken) = signal_port_with_ws_taken();
    let mut server = start(&dir, &signal_config(&dir, port, ""), &[]);

    // Act
    let (status, _) = server.wait_exit(SHUTDOWN_LIMIT);

    // Assert
    let logs = server.logs();
    assert!(!status.success(), "{logs}");
    assert!(logs.contains("signal server failed to start"), "{logs}");
    assert!(
        logs.contains(&format!("cannot listen on tcp port {}", port + 2)),
        "{logs}"
    );
    assert!(!logs.contains("\"ready\""), "{logs}");
}

#[test]
fn disabled_signal_server_starts_without_signal_ports() {
    // Arrange
    let dir = workdir("signal-disabled");
    let port = free_signal_port();
    let config = config_with_signal(&dir, "", &format!("enabled = false\nport = {port}"));

    // Act
    let mut server = start(&dir, &config, &[]);
    let addr = server.listening("client_api");

    // Assert
    wait_ready(addr);
    assert!(
        TcpStream::connect((LOCALHOST, port)).is_err(),
        "signal port open"
    );
    let (status, _) = server.terminate();
    let logs = server.logs();
    assert_eq!(status.code(), Some(0), "{logs}");
    assert!(logs.contains("signal server disabled"), "{logs}");
}

#[test]
fn config_check_reports_busy_signal_ports() {
    // Arrange
    let dir = workdir("signal-check");
    let port = free_signal_port();
    std::fs::write(dir.join("deskhaven.toml"), signal_config(&dir, port, "")).unwrap();
    let any = |port| SocketAddr::from(([0, 0, 0, 0], port));

    // Act
    let free = config_check(&dir);
    let nat_taken = {
        let _taken = TcpListener::bind(any(port - 1)).unwrap();
        config_check(&dir)
    };
    let udp_taken = {
        let _taken = UdpSocket::bind(any(port)).unwrap();
        config_check(&dir)
    };

    // Assert
    assert!(
        free.status.success(),
        "{}",
        String::from_utf8_lossy(&free.stderr)
    );
    let nat = String::from_utf8_lossy(&nat_taken.stderr);
    assert!(!nat_taken.status.success(), "{nat}");
    assert!(
        nat.contains("signal NAT test (port-1): cannot bind TCP"),
        "{nat}"
    );
    let udp = String::from_utf8_lossy(&udp_taken.stderr);
    assert!(!udp_taken.status.success(), "{udp}");
    assert!(
        udp.contains(&format!("signal.port: cannot bind UDP 0.0.0.0:{port}")),
        "{udp}"
    );
}

/// With `signal.ws_listen` set, `config check` probes that address instead of port+2.
#[test]
fn config_check_probes_signal_ws_listen_instead_of_port_plus_two() {
    // Arrange
    let dir = workdir("signal-check-ws-listen");
    let port = free_signal_port();
    let ws_port = free_signal_port();
    let base = config_with_signal(
        &dir,
        "",
        &format!("port = {port}\nws_listen = \"127.0.0.1:{ws_port}\""),
    );
    std::fs::write(dir.join("deskhaven.toml"), base).unwrap();

    // Act
    let plus_two_taken = {
        let _taken = TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], port + 2))).unwrap();
        config_check(&dir)
    };
    let ws_taken = {
        let _taken = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], ws_port))).unwrap();
        config_check(&dir)
    };

    // Assert
    let stderr = String::from_utf8_lossy(&plus_two_taken.stderr);
    assert!(plus_two_taken.status.success(), "{stderr}");
    let stderr = String::from_utf8_lossy(&ws_taken.stderr);
    assert!(!ws_taken.status.success(), "{stderr}");
    let expected = format!("signal.ws_listen: cannot bind TCP 127.0.0.1:{ws_port}");
    assert!(stderr.contains(&expected), "{stderr}");
}

/// A holder of a signal port on IPv6 only (here `[::1]`) does not collide with the `0.0.0.0`
/// probe; the `[::]` probe reports it. Skipped where the host has no IPv6 loopback.
#[test]
fn config_check_reports_an_ipv6_only_holder_of_a_signal_port() {
    // Arrange
    let dir = workdir("signal-check-v6");
    let port = free_signal_port();
    std::fs::write(dir.join("deskhaven.toml"), signal_config(&dir, port, "")).unwrap();
    let Ok(_taken) = TcpListener::bind(("::1", port + 2)) else {
        eprintln!("no IPv6 loopback; skipped");
        return;
    };

    // Act
    let out = config_check(&dir);

    // Assert
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{stderr}");
    let expected = format!(
        "signal WebSocket (port+2): cannot bind TCP [::]:{}",
        port + 2
    );
    assert!(stderr.contains(&expected), "{stderr}");
}

// --- protocol helpers ----------------------------------------------------------------------

fn rv(set: impl FnOnce(&mut RendezvousMessage)) -> RendezvousMessage {
    let mut msg = RendezvousMessage::new();
    set(&mut msg);
    msg
}

fn base64_decode(text: &str) -> Vec<u8> {
    use base64_022::Engine as _;
    base64_022::engine::general_purpose::STANDARD
        .decode(text)
        .unwrap()
}

fn punch_request(licence_key: &str) -> RendezvousMessage {
    rv(|m| {
        m.set_punch_hole_request(PunchHoleRequest {
            id: TARGET.to_owned(),
            licence_key: licence_key.to_owned(),
            nat_type: NatType::ASYMMETRIC.into(),
            version: CLIENT_VERSION.to_owned(),
            ..Default::default()
        })
    })
}

fn udp(socket: &UdpSocket, msg: &RendezvousMessage) -> RendezvousMessage {
    socket.send(&msg.write_to_bytes().unwrap()).unwrap();
    udp_recv(socket)
}

fn udp_recv(socket: &UdpSocket) -> RendezvousMessage {
    let mut buf = vec![0; 64 * 1024];
    let n = socket.recv(&mut buf).unwrap();
    RendezvousMessage::parse_from_bytes(&buf[..n]).unwrap()
}

fn register_pk(socket: &UdpSocket, uuid: &[u8], pk: &[u8]) -> register_pk_response::Result {
    let msg = rv(|m| {
        m.set_register_pk(RegisterPk {
            id: TARGET.to_owned(),
            uuid: uuid.to_vec().into(),
            pk: pk.to_vec().into(),
            ..Default::default()
        })
    });
    match udp(socket, &msg).union {
        Some(rendezvous_message::Union::RegisterPkResponse(r)) => r.result.enum_value().unwrap(),
        other => panic!("expected RegisterPkResponse, got {other:?}"),
    }
}

/// One length-framed TCP connection (hbb_common `BytesCodec`, as RustDesk clients use).
struct Tcp(Framed<tokio::net::TcpStream, BytesCodec>);

impl Tcp {
    async fn connect(port: u16) -> Self {
        let stream = tokio::net::TcpStream::connect((LOCALHOST, port))
            .await
            .unwrap();
        Self(Framed::new(stream, BytesCodec::new()))
    }

    async fn send(&mut self, msg: &RendezvousMessage) {
        self.0
            .send(msg.write_to_bytes().unwrap().into())
            .await
            .unwrap();
    }

    /// The next message, skipping the server's KeyExchange offer like a client that does not
    /// secure the connection (main port, M1-07).
    async fn recv(&mut self) -> RendezvousMessage {
        loop {
            let frame = tokio::time::timeout(NET_TIMEOUT, self.0.next())
                .await
                .unwrap();
            let msg = RendezvousMessage::parse_from_bytes(&frame.unwrap().unwrap()).unwrap();
            if !msg.has_key_exchange() {
                return msg;
            }
        }
    }
}

async fn request(port: u16, msg: &RendezvousMessage) -> RendezvousMessage {
    let mut conn = Tcp::connect(port).await;
    conn.send(msg).await;
    conn.recv().await
}

/// The target's pk as the server signs it: RequestRelay from a controller, forwarded to the
/// target over UDP, answered by the target over TCP, delivered to the controller.
async fn relayed_pk(port: u16, licence_key: &str, target: &UdpSocket) -> Vec<u8> {
    let mut controller = Tcp::connect(port).await;
    controller
        .send(&rv(|m| {
            m.set_request_relay(RequestRelay {
                id: TARGET.to_owned(),
                licence_key: licence_key.to_owned(),
                relay_server: PINNED_RELAY.to_owned(),
                ..Default::default()
            })
        }))
        .await;
    let fwd = match udp_recv(target).union {
        Some(rendezvous_message::Union::RequestRelay(fwd)) => fwd,
        other => panic!("expected RequestRelay, got {other:?}"),
    };
    let mut answer = RelayResponse {
        socket_addr: fwd.socket_addr,
        relay_server: PINNED_RELAY.to_owned(),
        version: CLIENT_VERSION.to_owned(),
        ..Default::default()
    };
    answer.set_id(TARGET.to_owned());
    let mut target_tcp = Tcp::connect(port).await;
    target_tcp.send(&rv(|m| m.set_relay_response(answer))).await;
    match controller.recv().await.union {
        Some(rendezvous_message::Union::RelayResponse(res)) => res.pk().to_vec(),
        other => panic!("expected RelayResponse, got {other:?}"),
    }
}
