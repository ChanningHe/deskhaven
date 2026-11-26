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

/// Mirrors the server's `STREAM_HEARTBEAT`.
const HEARTBEAT: Duration = Duration::from_secs(10);

async fn assert_served(conn: &mut impl Rendezvous, server: &TestServer) {
    conn.punch_hole_request(server.punch_req("kx-no-such-peer"))
        .await;
    let res = expect_msg!(conn.recv().await, PunchHoleResponse);
    assert_eq!(res.failure.enum_value(), Ok(ID_NOT_EXIST));
}

async fn offer(server: &TestServer) -> KeyExchange {
    read_offer(&mut server.tcp().await).await
}

/// Changes the message's last byte of a combined-mode signature, leaving the signature alone.
fn tamper_signed_pk(signed: &Bytes, edit: impl FnOnce(&mut u8)) -> Bytes {
    let mut out = signed.to_vec();
    edit(out.last_mut().unwrap());
    out.into()
}

#[tokio::test]
async fn v1_client_completes_the_handshake_and_exchanges_encrypted_frames() {
    let server = TestServer::start().await;
    let mut conn = server.tcp().await;

    let picked = secure(&mut conn, &server.public_key, KxClient::Current).await;

    assert_eq!(picked, Ok(1));
    assert!(conn.stream().is_secured());
    assert_eq!(
        conn.register_pk("kx-roundtrip", &PeerKeys::random()).await,
        OK
    );
    assert_served(&mut conn, &server).await;
    server.stop().await;
}

#[tokio::test]
async fn legacy_v0_client_completes_the_handshake_with_one_key() {
    let server = TestServer::start().await;
    let mut conn = server.tcp().await;

    let picked = secure(&mut conn, &server.public_key, KxClient::Legacy).await;

    assert_eq!(picked, Ok(0));
    assert_eq!(
        conn.register_pk("kx-roundtrip", &PeerKeys::random()).await,
        OK
    );
    assert_served(&mut conn, &server).await;
    server.stop().await;
}

#[tokio::test]
async fn offer_is_sent_at_once_with_a_fresh_flagged_key_per_connection() {
    let server = TestServer::start().await;

    let first = timeout(QUIET, offer(&server)).await.expect("sent at once");
    let second = offer(&server).await;

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

/// The key's high bit is inside the signature, so stripping it fails every client's check.
#[tokio::test]
async fn stripping_the_high_bit_fails_the_clients_signature_check() {
    let server = TestServer::start().await;
    let mut ex = offer(&server).await;

    ex.keys[0] = tamper_signed_pk(&ex.keys[0], |b| *b &= !SIGNED_PARAMS_FLAG);
    let stripped_and_dropped = KeyExchange {
        signed_params: Bytes::new(),
        ..ex.clone()
    };

    for client in [KxClient::Current, KxClient::Legacy] {
        for tampered in [&ex, &stripped_and_dropped] {
            let err = verify_offer(tampered, &server.public_key, client).unwrap_err();
            assert!(err.contains("Signature mismatch"), "{client:?}: {err}");
        }
    }
    server.stop().await;
}

#[tokio::test]
async fn dropping_signed_params_or_the_version_fails_the_clients_check() {
    let server = TestServer::start().await;
    let ex = offer(&server).await;

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

    for tampered in [&dropped, &downgraded, &both] {
        assert!(
            verify_offer(tampered, &server.public_key, KxClient::Current).is_err(),
            "{tampered:?}"
        );
    }
    server.stop().await;
}

#[tokio::test]
async fn signed_params_of_another_connection_or_key_fail_the_clients_check() {
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

    let replayed = KeyExchange {
        signed_params: other.signed_params.clone(),
        ..ex.clone()
    };
    let foreign = KeyExchange {
        signed_params: sign::sign(&payload, &foreign_sk).into(),
        ..ex.clone()
    };

    for tampered in [&replayed, &foreign] {
        assert!(verify_offer(tampered, &server.public_key, KxClient::Current).is_err());
    }
    server.stop().await;
}

#[tokio::test]
async fn reply_picking_a_version_above_the_advertised_one_closes_the_connection() {
    let server = TestServer::start().await;
    let mut conn = server.tcp().await;
    let ex = read_offer(&mut conn).await;
    let their_pk = verify_offer(&ex, &server.public_key, KxClient::Current).unwrap();

    conn.send(&reply(their_pk, ex.version + 1).msg).await;

    conn.assert_closed().await;
    server.stop().await;
}

#[tokio::test]
async fn malformed_replies_close_the_connection() {
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

        conn.send(&rv(|m| {
            m.set_key_exchange(KeyExchange {
                keys: keys.clone(),
                version: 1,
                ..Default::default()
            })
        }))
        .await;

        conn.assert_closed().await;
    }
    server.stop().await;
}

#[tokio::test]
async fn plaintext_frame_on_a_secured_connection_closes_it() {
    let server = TestServer::start().await;
    let mut conn = secured_tcp(&server).await;
    let plain = rv(|m| m.set_punch_hole_request(server.punch_req("kx-no-such-peer")));

    conn.stream()
        .send_bytes(plain.write_to_bytes().unwrap().into())
        .await
        .unwrap();

    conn.assert_closed().await;
    server.stop().await;
}

#[tokio::test]
async fn require_kx_v1_refuses_v0_but_serves_v1_and_plaintext_clients() {
    let server = TestServer::start_with(|cfg| SignalConfig {
        require_kx_v1: true,
        ..cfg
    })
    .await;
    let mut legacy = server.tcp().await;
    let mut current = server.tcp().await;
    let mut plain = server.tcp().await;

    let legacy_picked = secure(&mut legacy, &server.public_key, KxClient::Legacy).await;
    let current_picked = secure(&mut current, &server.public_key, KxClient::Current).await;

    assert_eq!(legacy_picked, Ok(0));
    legacy.assert_closed().await;
    assert_eq!(current_picked, Ok(1));
    assert_served(&mut current, &server).await;
    assert_served(&mut plain, &server).await;
    server.stop().await;
}

#[tokio::test]
async fn require_kx_v1_can_be_switched_at_runtime() {
    let server = TestServer::start().await;

    for (on, refused) in [(true, true), (false, false)] {
        server.handle.set_require_kx_v1(on).unwrap();
        let mut legacy = server.tcp().await;
        secure(&mut legacy, &server.public_key, KxClient::Legacy)
            .await
            .unwrap();

        if refused {
            legacy.assert_closed().await;
        } else {
            assert_served(&mut legacy, &server).await;
        }
    }
    server.stop().await;
}

#[tokio::test]
async fn punch_hole_response_from_another_connection_uses_the_controllers_cipher() {
    const TARGET: &str = "kx-cross-punch";
    let server = TestServer::start().await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let mut controller = secured_tcp(&server).await;
    controller
        .punch_hole_request(server.punch_req(TARGET))
        .await;
    let ph = expect_msg!(target.recv().await, PunchHole);
    let mut answer = secured_tcp(&server).await;

    let sent = punch_hole_sent(&ph.socket_addr, TARGET, "");
    answer.send(&rv(|m| m.set_punch_hole_sent(sent))).await;

    let res = expect_msg!(controller.recv().await, PunchHoleResponse);
    assert_eq!(decode_addr(&res.socket_addr), answer.local_addr());
    server.stop().await;
}

#[tokio::test]
async fn relay_response_from_another_connection_uses_the_controllers_cipher() {
    const TARGET: &str = "kx-cross-relay";
    let server = TestServer::start().await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let mut controller = secured_tcp(&server).await;
    controller
        .request_relay(support::request_relay(TARGET, &server.licence_key))
        .await;
    let forwarded = expect_msg!(target.recv().await, RequestRelay);
    let mut answer = secured_tcp(&server).await;

    let res = relay_response(&forwarded, TARGET);
    answer.send(&rv(|m| m.set_relay_response(res))).await;

    let res = expect_msg!(controller.recv().await, RelayResponse);
    assert_eq!(res.uuid, forwarded.uuid);
    server.stop().await;
}

#[tokio::test]
async fn secured_tcp_mode_peer_receives_forwarded_messages_under_its_own_key() {
    const TARGET: &str = "kx-tcp-mode-peer";
    let server = TestServer::start().await;
    let mut target = secured_tcp(&server).await;
    assert_eq!(target.register_pk(TARGET, &PeerKeys::random()).await, OK);
    let mut controller = secured_tcp(&server).await;

    controller
        .punch_hole_request(server.punch_req(TARGET))
        .await;
    let ph = expect_msg!(target.recv().await, PunchHole);
    let mut answer = secured_tcp(&server).await;
    let sent = punch_hole_sent(&ph.socket_addr, TARGET, "");
    answer.send(&rv(|m| m.set_punch_hole_sent(sent))).await;

    assert_eq!(decode_addr(&ph.socket_addr), controller.local_addr());
    let res = expect_msg!(controller.recv().await, PunchHoleResponse);
    assert_eq!(decode_addr(&res.socket_addr), answer.local_addr());
    server.stop().await;
}

#[tokio::test]
async fn client_that_skips_the_key_exchange_is_served_in_plaintext() {
    const TARGET: &str = "kx-plain-peer";
    let server = TestServer::start().await;
    let mut target = server.tcp().await;
    let mut controller = server.tcp().await;

    let registered = target.register_pk(TARGET, &PeerKeys::random()).await;
    controller
        .punch_hole_request(server.punch_req(TARGET))
        .await;
    let ph = expect_msg!(target.recv().await, PunchHole);

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

#[tokio::test(start_paused = true)]
async fn heartbeats_on_a_secured_connection_are_plaintext_empty_frames() {
    const TARGET: &str = "kx-heartbeat-peer";
    let server = TestServer::start().await;
    let mut target = secured_tcp(&server).await;
    assert_eq!(target.register_pk(TARGET, &PeerKeys::random()).await, OK);

    // The registration answer proves the server caught up; read the heartbeat raw, below the
    // cipher, with the clock held still.
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
    // A closed connection answers at once, so stop on anything but silence (the paused clock
    // would never advance).
    let started = hbb_common::tokio::time::Instant::now();
    while started.elapsed() < REG_TIMEOUT * 2 {
        match target.recv_within(QUIET).await {
            Recv::Quiet => {}
            other => panic!("after {:?}: {other:?}", started.elapsed()),
        }
    }

    assert!(target.heartbeats() >= 4, "{}", target.heartbeats());
    let mut controller = server.tcp().await;
    controller
        .punch_hole_request(server.punch_req(TARGET))
        .await;
    let ph = expect_msg!(target.recv().await, PunchHole);
    assert_eq!(decode_addr(&ph.socket_addr), controller.local_addr());
    server.stop().await;
}

#[tokio::test]
async fn sealed_empty_frame_from_a_registered_peer_keeps_the_cipher_in_step() {
    const TARGET: &str = "kx-sealed-empty";
    let server = TestServer::start().await;
    let mut target = secured_tcp(&server).await;
    let keys = PeerKeys::random();
    assert_eq!(target.register_pk(TARGET, &keys).await, OK);

    target.stream().send_raw(Vec::new()).await.unwrap();
    let again = target.register_pk(TARGET, &keys).await;

    assert_eq!(again, OK);
    server.stop().await;
}

#[tokio::test]
async fn empty_first_frame_is_handled_as_plain_text() {
    let server = TestServer::start().await;
    let mut conn = server.tcp().await;
    read_offer(&mut conn).await;

    conn.stream().send_bytes(Bytes::new()).await.unwrap();

    conn.assert_closed().await;
    server.stop().await;
}

#[tokio::test]
async fn websocket_and_nat_port_get_no_key_exchange() {
    let server = TestServer::start().await;
    let mut ws = server.ws().await;
    let mut nat = server.nat().await;

    let ws_first = ws.recv_within(QUIET).await;
    let nat_first = nat.recv_within(QUIET).await;

    assert!(matches!(ws_first, Recv::Quiet), "{ws_first:?}");
    assert!(matches!(nat_first, Recv::Quiet), "{nat_first:?}");
    assert!(nat.key_exchanges().is_empty());
    assert!(nat.test_nat().await.port > 0);
    assert_served(&mut ws, &server).await;
    server.stop().await;
}
