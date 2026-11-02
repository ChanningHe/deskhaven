//! `signal.require_kx_v1` (M1-07) reaching the running signal server.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use base64_022::Engine as _;
use deskhaven_config::settings::Snapshot;
use hbb_common::bytes_codec::BytesCodec;
use hbb_common::futures_util::{SinkExt as _, StreamExt as _};
use hbb_common::protobuf::Message as _;
use hbb_common::rendezvous_proto::{KeyExchange, RendezvousMessage, rendezvous_message};
use hbb_common::tokio_util::codec::Framed;
use serde_json::json;
use sodiumoxide::crypto::{box_, secretbox, sign};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio::time::{Instant, sleep, timeout};
use tokio_util::sync::CancellationToken;

use super::tests::{POLL, STEP, memory_store, start_launcher, with};

/// How long a refused connection takes at most to be closed after the reply.
const QUIET: Duration = Duration::from_millis(500);

/// Runs the KeyExchange of a client from before Kx v1 (version 0 reply) against the server
/// on `port`; `true` if the server keeps the connection open afterwards, `false` if it closes
/// it (refused).
async fn legacy_exchange_is_served(port: u16, server_pk: &sign::PublicKey) -> bool {
    let stream = TcpStream::connect((IpAddr::V4(Ipv4Addr::LOCALHOST), port))
        .await
        .unwrap();
    let mut conn = Framed::new(stream, BytesCodec::new());
    let first = timeout(STEP, conn.next()).await.unwrap().unwrap().unwrap();
    let offer = match RendezvousMessage::parse_from_bytes(&first).unwrap().union {
        Some(rendezvous_message::Union::KeyExchange(ex)) => ex,
        other => panic!("expected the KeyExchange offer, got {other:?}"),
    };
    let their_pk: [u8; 32] = sign::verify(&offer.keys[0], server_pk)
        .unwrap()
        .try_into()
        .unwrap();
    let (our_pk, our_sk) = box_::gen_keypair();
    let nonce = box_::Nonce([0u8; box_::NONCEBYTES]);
    let sealed = box_::seal(
        &secretbox::gen_key().0,
        &nonce,
        &box_::PublicKey(their_pk),
        &our_sk,
    );
    let mut reply = RendezvousMessage::new();
    reply.set_key_exchange(KeyExchange {
        keys: vec![our_pk.0.to_vec().into(), sealed.into()],
        ..Default::default()
    });
    conn.send(reply.write_to_bytes().unwrap().into())
        .await
        .unwrap();
    match timeout(QUIET, conn.next()).await {
        Err(_) => true,
        Ok(None | Some(Err(_))) => false,
        Ok(Some(Ok(frame))) => panic!("unexpected frame of {} bytes", frame.len()),
    }
}

/// Switching `signal.require_kx_v1` on makes the running server refuse version 0 replies;
/// switching it off again serves them.
#[tokio::test]
async fn require_kx_v1_setting_reaches_the_running_server() {
    // Arrange
    let (tx, rx) = watch::channel(Arc::new(Snapshot::defaults()));
    let token = CancellationToken::new();
    let (running, port, key) = start_launcher(rx, memory_store(), &token).await;
    let pk = base64_022::engine::general_purpose::STANDARD
        .decode(key)
        .unwrap();
    let server_pk = sign::PublicKey::from_slice(&pk).unwrap();
    assert!(legacy_exchange_is_served(port, &server_pk).await);

    for on in [true, false] {
        // Act
        tx.send_replace(Arc::new(with(&[("signal.require_kx_v1", json!(on))])));

        // Assert
        let deadline = Instant::now() + STEP;
        while legacy_exchange_is_served(port, &server_pk).await == on {
            assert!(
                Instant::now() < deadline,
                "require_kx_v1 = {on} never applied"
            );
            sleep(POLL).await;
        }
    }
    token.cancel();
    timeout(STEP, running.task).await.unwrap().unwrap();
}
