//! M1-07: KeyExchange on rendezvous TCP connections (07 §4).
//!
//! Right after accepting a TCP connection on the main port the server sends
//! `KeyExchange{keys: [sign(S_pk)], version: 1, signed_params: sign("rdkx-params" ||
//! KxParams{S_pk, 1})}` with a per-connection X25519 key whose bit 255 is set. Logged-in
//! controllers and TCP-mode controlled clients (`secure_tcp`) answer with their own key and a
//! sealed symmetric key; from then on every non-empty frame is a secretbox under one key (v0)
//! or one key per direction (v1). Clients that do not run the exchange skip the offer and
//! keep talking in plain text. Upstream hbbs sent nothing, so logged-in clients waited out
//! their 18 s read timeout and then failed ("Failed to secure tcp").

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use deskhaven_signal::SignalConfig;
use hbb_common::{
    bytes::Bytes,
    protobuf::Message as _,
    rendezvous_proto::{
        KeyExchange, KxParams, punch_hole_response::Failure::ID_NOT_EXIST,
        register_pk_response::Result::OK,
    },
    tcp::KX_PARAMS_DOMAIN,
    tokio::{
        self,
        time::{Duration, timeout},
    },
};
use sodiumoxide::crypto::sign;
use support::{
    Conn, KxClient, PeerKeys, QUIET, REG_TIMEOUT, Recv, Rendezvous, SIGNED_PARAMS_FLAG, TestServer,
    decode_addr, expect_msg, io, punch_hole_sent, read_offer, relay_response, reply, rv, secure,
    secured_tcp, verify_offer,
};

/// The server's heartbeat period on a registered connection (`STREAM_HEARTBEAT`).
const HEARTBEAT: Duration = Duration::from_secs(10);

/// A punch hole for an unknown id is answered ID_NOT_EXIST on the same connection.
async fn assert_served(conn: &mut impl Rendezvous, server: &TestServer) {
    conn.punch_hole_request(server.punch_req("kx-no-such-peer"))
        .await;
    let res = expect_msg!(conn.recv().await, PunchHoleResponse);
    assert_eq!(res.failure.enum_value(), Ok(ID_NOT_EXIST));
}

/// The offer of a fresh main-port connection.
async fn offer(server: &TestServer) -> KeyExchange {
    read_offer(&mut server.tcp().await).await
}

/// `signed` (a combined-mode signature: 64-byte signature, then the message) with the
/// message's last byte changed by `edit`, the signature left alone.
fn tamper_signed_pk(signed: &Bytes, edit: impl FnOnce(&mut u8)) -> Bytes {
    let mut out = signed.to_vec();
    edit(out.last_mut().unwrap());
    out.into()
}

/// A current (v1) client completes the exchange and both directions are encrypted: answers
/// decrypt under the client's receiving key, and the server decrypts what the client seals.
/// Two exchanges each way (the RegisterPkResponse from the connection's own sink, the
/// PunchHoleResponse through `tcp_punch`) keep the nonce counters in step.
#[tokio::test]
async fn v1_client_completes_the_handshake_and_exchanges_encrypted_frames() {
    // Arrange
    let server = TestServer::start().await;
    let mut conn = server.tcp().await;

    // Act
    let picked = secure(&mut conn, &server.public_key, KxClient::Current).await;

    // Assert
    assert_eq!(picked, Ok(1));
    assert!(conn.stream().is_secured());
    assert_eq!(
        conn.register_pk("kx-roundtrip", &PeerKeys::random()).await,
        OK
    );
    assert_served(&mut conn, &server).await;
    server.stop().await;
}

/// A client from before Kx v1 (checks only `keys[0]`, replies without a version) completes the
/// exchange too and both sides use the one key (v0).
#[tokio::test]
async fn legacy_v0_client_completes_the_handshake_with_one_key() {
    // Arrange
    let server = TestServer::start().await;
    let mut conn = server.tcp().await;

    // Act
    let picked = secure(&mut conn, &server.public_key, KxClient::Legacy).await;

    // Assert
    assert_eq!(picked, Ok(0));
    assert_eq!(
        conn.register_pk("kx-roundtrip", &PeerKeys::random()).await,
        OK
    );
    assert_served(&mut conn, &server).await;
    server.stop().await;
}

/// The offer comes unprompted at once, advertises v1, carries a flagged key that both client
/// generations accept, and every connection gets its own ephemeral key.
#[tokio::test]
async fn offer_is_sent_at_once_with_a_fresh_flagged_key_per_connection() {
    // Arrange
    let server = TestServer::start().await;

    // Act
    let first = timeout(QUIET, offer(&server)).await.expect("sent at once");
    let second = offer(&server).await;

    // Assert
    for ex in [&first, &second] {
        assert_eq!(ex.version, 1);
        let pk = verify_offer(ex, &server.public_key, KxClient::Current).unwrap();
        assert_eq!(pk[31] & SIGNED_PARAMS_FLAG, SIGNED_PARAMS_FLAG);
        assert_eq!(
            verify_offer(ex, &server.public_key, KxClient::Legacy),
            Ok(pk)
        );
    }
    assert_ne!(first.keys[0], second.keys[0]);
    server.stop().await;
}

/// Downgrade by stripping the key's high bit (so a v1 client would not ask for the signed
/// parameters): the bit is inside the signature, so the client's check fails for every
/// client generation.
#[tokio::test]
async fn stripping_the_high_bit_fails_the_clients_signature_check() {
    // Arrange
    let server = TestServer::start().await;
    let mut ex = offer(&server).await;

    // Act
    ex.keys[0] = tamper_signed_pk(&ex.keys[0], |b| *b &= !SIGNED_PARAMS_FLAG);
    let stripped_and_dropped = KeyExchange {
        signed_params: Bytes::new(),
        ..ex.clone()
    };

    // Assert
    for client in [KxClient::Current, KxClient::Legacy] {
        for tampered in [&ex, &stripped_and_dropped] {
            let err = verify_offer(tampered, &server.public_key, client).unwrap_err();
            assert!(err.contains("Signature mismatch"), "{client:?}: {err}");
        }
    }
    server.stop().await;
}

/// Downgrade by dropping `signed_params` or lowering the advertised version: the signed key's
/// high bit still demands the parameters, and they sign the version, so a current client
/// refuses before replying.
#[tokio::test]
async fn dropping_signed_params_or_the_version_fails_the_clients_check() {
    // Arrange
    let server = TestServer::start().await;
    let ex = offer(&server).await;

    // Act
    let dropped = KeyExchange {
        signed_params: Bytes::new(),
        ..ex.clone()
    };
    let downgraded = KeyExchange {
        version: 0,
        ..ex.clone()
    };
    let both = KeyExchange {
        version: 0,
        signed_params: Bytes::new(),
        ..ex.clone()
    };

    // Assert
    for tampered in [&dropped, &downgraded, &both] {
        assert!(
            verify_offer(tampered, &server.public_key, KxClient::Current).is_err(),
            "{tampered:?}"
        );
    }
    server.stop().await;
}

/// Signed parameters replayed from another connection (or signed by another key) do not
/// match this connection's key.
#[tokio::test]
async fn signed_params_of_another_connection_or_key_fail_the_clients_check() {
    // Arrange
    let server = TestServer::start().await;
    let ex = offer(&server).await;
    let other = offer(&server).await;
    let (_, foreign_sk) = sign::gen_keypair();
    let mut payload = KX_PARAMS_DOMAIN.to_vec();
    payload.extend(
        KxParams {
            pk: sign::verify(&ex.keys[0], &server.public_key)
                .unwrap()
                .into(),
            version: 1,
            ..Default::default()
        }
        .write_to_bytes()
        .unwrap(),
    );

    // Act
    let replayed = KeyExchange {
        signed_params: other.signed_params.clone(),
        ..ex.clone()
    };
    let foreign = KeyExchange {
        signed_params: sign::sign(&payload, &foreign_sk).into(),
        ..ex.clone()
    };

    // Assert
    for tampered in [&replayed, &foreign] {
        assert!(verify_offer(tampered, &server.public_key, KxClient::Current).is_err());
    }
    server.stop().await;
}

/// The server holds the client to `picked <= advertised`: a reply picking a version the server
/// never offered closes the connection.
#[tokio::test]
async fn reply_picking_a_version_above_the_advertised_one_closes_the_connection() {
    // Arrange
    let server = TestServer::start().await;
    let mut conn = server.tcp().await;
    let ex = read_offer(&mut conn).await;
    let their_pk = verify_offer(&ex, &server.public_key, KxClient::Current).unwrap();

    // Act
    conn.send(&reply(their_pk, ex.version + 1).msg).await;

    // Assert
    conn.assert_closed().await;
    server.stop().await;
}

/// A KeyExchange reply the server cannot use (wrong number of keys, a public key of the wrong
/// length, a box that does not open with this connection's key) closes the connection.
#[tokio::test]
async fn malformed_replies_close_the_connection() {
    // Arrange
    let server = TestServer::start().await;
    let other_pk =
        verify_offer(&offer(&server).await, &server.public_key, KxClient::Current).unwrap();
    let valid = reply(other_pk, 1).msg;
    let keys = valid.key_exchange().keys.clone();
    let malformed = [
        vec![keys[0].clone()],
        vec![keys[0].slice(..31), keys[1].clone()],
        // Sealed to another connection's key.
        keys.clone(),
        vec![keys[0].clone(), keys[1].slice(..47)],
    ];

    for keys in malformed {
        let mut conn = server.tcp().await;
        read_offer(&mut conn).await;

        // Act
        conn.send(&rv(|m| {
            m.set_key_exchange(KeyExchange {
                keys: keys.clone(),
                version: 1,
                ..Default::default()
            })
        }))
        .await;

        // Assert
        conn.assert_closed().await;
    }
    server.stop().await;
}

/// Once secured, a frame that does not decrypt (here: sent in plain text) closes the
/// connection; nothing is processed in the clear.
#[tokio::test]
async fn plaintext_frame_on_a_secured_connection_closes_it() {
    // Arrange
    let server = TestServer::start().await;
    let mut conn = secured_tcp(&server).await;
    let plain = rv(|m| m.set_punch_hole_request(server.punch_req("kx-no-such-peer")));

    // Act
    conn.stream()
        .send_bytes(plain.write_to_bytes().unwrap().into())
        .await
        .unwrap();

    // Assert
    conn.assert_closed().await;
    server.stop().await;
}

/// `require_kx_v1`: a v0 reply closes the connection; v1 clients and clients that skip the
/// exchange are served as before.
#[tokio::test]
async fn require_kx_v1_refuses_v0_but_serves_v1_and_plaintext_clients() {
    // Arrange
    let server = TestServer::start_with(|cfg| SignalConfig {
        require_kx_v1: true,
        ..cfg
    })
    .await;
    let mut legacy = server.tcp().await;
    let mut current = server.tcp().await;
    let mut plain = server.tcp().await;

    // Act
    let legacy_picked = secure(&mut legacy, &server.public_key, KxClient::Legacy).await;
    let current_picked = secure(&mut current, &server.public_key, KxClient::Current).await;

    // Assert
    assert_eq!(legacy_picked, Ok(0));
    legacy.assert_closed().await;
    assert_eq!(current_picked, Ok(1));
    assert_served(&mut current, &server).await;
    assert_served(&mut plain, &server).await;
    server.stop().await;
}

/// `require_kx_v1` follows `SignalHandle::set_require_kx_v1` for the exchanges that complete
/// after the change.
#[tokio::test]
async fn require_kx_v1_can_be_switched_at_runtime() {
    // Arrange
    let server = TestServer::start().await;

    for (on, refused) in [(true, true), (false, false)] {
        // Act
        server.handle.set_require_kx_v1(on).unwrap();
        let mut legacy = server.tcp().await;
        secure(&mut legacy, &server.public_key, KxClient::Legacy)
            .await
            .unwrap();

        // Assert
        if refused {
            legacy.assert_closed().await;
        } else {
            assert_served(&mut legacy, &server).await;
        }
    }
    server.stop().await;
}

/// Cross-connection forwarding: the target's PunchHoleSent arrives on the target's own secured
/// connection, and the server's task for that connection writes the PunchHoleResponse to the
/// controller's stored sink, sealed under the controller's key (it decrypts there).
#[tokio::test]
async fn punch_hole_response_from_another_connection_uses_the_controllers_cipher() {
    const TARGET: &str = "kx-cross-punch";
    // Arrange
    let server = TestServer::start().await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let mut controller = secured_tcp(&server).await;
    controller
        .punch_hole_request(server.punch_req(TARGET))
        .await;
    let ph = expect_msg!(target.recv().await, PunchHole);
    let mut answer = secured_tcp(&server).await;

    // Act
    let sent = punch_hole_sent(&ph.socket_addr, TARGET, "");
    answer.send(&rv(|m| m.set_punch_hole_sent(sent))).await;

    // Assert
    let res = expect_msg!(controller.recv().await, PunchHoleResponse);
    assert_eq!(decode_addr(&res.socket_addr), answer.local_addr());
    server.stop().await;
}

/// The same for a relayed connection: the target's RelayResponse, sent on its own secured
/// connection, reaches the controller sealed under the controller's key.
#[tokio::test]
async fn relay_response_from_another_connection_uses_the_controllers_cipher() {
    const TARGET: &str = "kx-cross-relay";
    // Arrange
    let server = TestServer::start().await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let mut controller = secured_tcp(&server).await;
    controller
        .request_relay(support::request_relay(TARGET, &server.licence_key))
        .await;
    let forwarded = expect_msg!(target.recv().await, RequestRelay);
    let mut answer = secured_tcp(&server).await;

    // Act
    let res = relay_response(&forwarded, TARGET);
    answer.send(&rv(|m| m.set_relay_response(res))).await;

    // Assert
    let res = expect_msg!(controller.recv().await, RelayResponse);
    assert_eq!(res.uuid, forwarded.uuid);
    server.stop().await;
}

/// Both sides logged in, the target in TCP mode: the target registers over its secured
/// connection and receives the PunchHole there under its own key, while the controller's
/// answer comes under the controller's.
#[tokio::test]
async fn secured_tcp_mode_peer_receives_forwarded_messages_under_its_own_key() {
    const TARGET: &str = "kx-tcp-mode-peer";
    // Arrange
    let server = TestServer::start().await;
    let mut target = secured_tcp(&server).await;
    assert_eq!(target.register_pk(TARGET, &PeerKeys::random()).await, OK);
    let mut controller = secured_tcp(&server).await;

    // Act
    controller
        .punch_hole_request(server.punch_req(TARGET))
        .await;
    let ph = expect_msg!(target.recv().await, PunchHole);
    let mut answer = secured_tcp(&server).await;
    let sent = punch_hole_sent(&ph.socket_addr, TARGET, "");
    answer.send(&rv(|m| m.set_punch_hole_sent(sent))).await;

    // Assert
    assert_eq!(decode_addr(&ph.socket_addr), controller.local_addr());
    let res = expect_msg!(controller.recv().await, PunchHoleResponse);
    assert_eq!(decode_addr(&res.socket_addr), answer.local_addr());
    server.stop().await;
}

/// A client that does not run the exchange (not logged in) skips the one offer and is served
/// in plain text as before, as controller and as TCP-mode peer.
#[tokio::test]
async fn client_that_skips_the_key_exchange_is_served_in_plaintext() {
    const TARGET: &str = "kx-plain-peer";
    // Arrange
    let server = TestServer::start().await;
    let mut target = server.tcp().await;
    let mut controller = server.tcp().await;

    // Act
    let registered = target.register_pk(TARGET, &PeerKeys::random()).await;
    controller
        .punch_hole_request(server.punch_req(TARGET))
        .await;
    let ph = expect_msg!(target.recv().await, PunchHole);

    // Assert
    assert_eq!(registered, OK);
    assert_eq!(decode_addr(&ph.socket_addr), controller.local_addr());
    assert_served(&mut controller, &server).await;
    for conn in [&target, &controller] {
        assert_eq!(conn.key_exchanges().len(), 1);
        let ex = &conn.key_exchanges()[0];
        assert!(verify_offer(ex, &server.public_key, KxClient::Current).is_ok());
    }
    server.stop().await;
}

/// Heartbeats on a secured registered connection stay empty plain frames (clients skip frames
/// of at most one byte when decrypting and answer them with plain empty frames), the server
/// does not try to decrypt those answers, and the cipher stays in step: the peer stays online
/// past REG_TIMEOUT and still decrypts what is forwarded to it. Paused clock.
#[tokio::test(start_paused = true)]
async fn heartbeats_on_a_secured_connection_are_plaintext_empty_frames() {
    const TARGET: &str = "kx-heartbeat-peer";
    // Arrange
    let server = TestServer::start().await;
    let mut target = secured_tcp(&server).await;
    assert_eq!(target.register_pk(TARGET, &PeerKeys::random()).await, OK);

    // Act: let the first heartbeat fall due (the registration answer proves the server is
    // caught up), then read it raw, below the cipher, with the clock held still.
    tokio::time::advance(HEARTBEAT + Duration::from_secs(1)).await;
    let raw = io(
        "a heartbeat",
        hbb_common::futures::StreamExt::next(&mut target.stream().0),
    )
    .await
    .expect("open")
    .expect("readable");
    assert!(raw.is_empty(), "heartbeat frame of {} bytes", raw.len());
    target.stream().send_bytes(Bytes::new()).await.unwrap();
    // Listen (answering heartbeats through the cipher layer) for twice REG_TIMEOUT. A closed
    // connection answers at once, so stop on anything but silence (the paused clock would
    // never advance).
    let started = hbb_common::tokio::time::Instant::now();
    while started.elapsed() < REG_TIMEOUT * 2 {
        match target.recv_within(QUIET).await {
            Recv::Quiet => {}
            other => panic!("after {:?}: {other:?}", started.elapsed()),
        }
    }

    // Assert
    assert!(target.heartbeats() >= 4, "{}", target.heartbeats());
    let mut controller = server.tcp().await;
    controller
        .punch_hole_request(server.punch_req(TARGET))
        .await;
    let ph = expect_msg!(target.recv().await, PunchHole);
    assert_eq!(decode_addr(&ph.socket_addr), controller.local_addr());
    server.stop().await;
}

/// An empty payload sealed by the client (a 16-byte frame, e.g. `send_raw(vec![])` on a secured
/// stream) is opened like any frame: it advances the receiving counter and then counts as an
/// empty frame (a heartbeat answer from a registered peer), so the connection stays open and
/// the next sealed frame still opens.
#[tokio::test]
async fn sealed_empty_frame_from_a_registered_peer_keeps_the_cipher_in_step() {
    const TARGET: &str = "kx-sealed-empty";
    // Arrange
    let server = TestServer::start().await;
    let mut target = secured_tcp(&server).await;
    let keys = PeerKeys::random();
    assert_eq!(target.register_pk(TARGET, &keys).await, OK);

    // Act
    target.stream().send_raw(Vec::new()).await.unwrap();
    let again = target.register_pk(TARGET, &keys).await;

    // Assert
    assert_eq!(again, OK);
    server.stop().await;
}

/// An empty first frame is not a KeyExchange reply: the connection falls back to plain text
/// and is handled as before M1-07, where an empty frame from an unregistered connection closes
/// it.
#[tokio::test]
async fn empty_first_frame_is_handled_as_plain_text() {
    // Arrange
    let server = TestServer::start().await;
    let mut conn = server.tcp().await;
    read_offer(&mut conn).await;

    // Act
    conn.stream().send_bytes(Bytes::new()).await.unwrap();

    // Assert
    conn.assert_closed().await;
    server.stop().await;
}

/// No offer on the websocket port (clients skip the exchange there: wss protects it) nor on
/// the NAT test port (clients never secure it); both are served in plain text as before.
#[tokio::test]
async fn websocket_and_nat_port_get_no_key_exchange() {
    // Arrange
    let server = TestServer::start().await;
    let mut ws = server.ws().await;
    let mut nat = server.nat().await;

    // Act
    let ws_first = ws.recv_within(QUIET).await;
    let nat_first = nat.recv_within(QUIET).await;

    // Assert
    assert!(matches!(ws_first, Recv::Quiet), "{ws_first:?}");
    assert!(matches!(nat_first, Recv::Quiet), "{nat_first:?}");
    assert!(nat.key_exchanges().is_empty());
    assert!(nat.test_nat().await.port > 0);
    assert_served(&mut ws, &server).await;
    server.stop().await;
}
