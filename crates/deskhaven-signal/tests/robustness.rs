//! After each malformed input the server must still answer a valid request.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use hbb_common::{
    protobuf::Message as _,
    tokio::{
        self,
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
        time::timeout,
    },
};
use support::{Conn, LOCALHOST, QUIET, Rendezvous, TcpClient, TestServer, register_peer};
use tokio_tungstenite::tungstenite::Message;

const GARBAGE: [u8; 32] = [0xff; 32];

/// A valid RegisterPeer cut short: a length-delimited field that runs past the end.
fn truncated_protobuf() -> Vec<u8> {
    let bytes = register_peer("123456789").write_to_bytes().unwrap();
    bytes[..bytes.len() - 3].to_vec()
}

#[tokio::test]
async fn udp_garbage_and_truncated_protobuf_get_no_answer() {
    let server = TestServer::start().await;
    let mut client = server.udp().await;

    client.send_raw(&GARBAGE).await;
    client.send_raw(&truncated_protobuf()).await;

    client.assert_silent().await;
    assert!(client.register_peer("still-serving").await.request_pk);
    server.stop().await;
}

#[tokio::test]
async fn tcp_garbage_first_frame_closes_the_connection() {
    let server = TestServer::start().await;

    for addr in [server.main_addr(), server.nat_addr()] {
        for payload in [GARBAGE.to_vec(), truncated_protobuf()] {
            let mut client = TcpClient::connect(addr).await;

            client.stream().send_raw(payload).await.unwrap();

            client.assert_closed().await;
        }
        assert!(
            TcpClient::connect(addr).await.test_nat().await.port > 0,
            "{addr}"
        );
    }
    server.stop().await;
}

#[tokio::test]
async fn websocket_text_frame_is_skipped_and_garbage_binary_closes() {
    let server = TestServer::start().await;
    let mut texting = server.ws().await;
    let mut garbling = server.ws().await;

    texting.send_frame(Message::Text("hello".into())).await;
    garbling
        .send_frame(Message::Binary(GARBAGE.to_vec().into()))
        .await;

    assert!(
        texting.test_nat().await.port > 0,
        "same connection still answers"
    );
    garbling.assert_closed().await;
    assert!(server.ws().await.test_nat().await.port > 0);
    server.stop().await;
}

/// Upstream handed loopback connections to the NAT port to an unauthenticated text console,
/// now removed.
#[tokio::test]
async fn loopback_console_commands_on_nat_port_get_no_output() {
    let server = TestServer::start().await;

    for cmd in ["h\n", "rs 192.0.2.66\n"] {
        let mut stream = TcpStream::connect(server.nat_addr()).await.unwrap();
        assert!(stream.local_addr().unwrap().ip().is_loopback());

        stream.write_all(cmd.as_bytes()).await.unwrap();
        let mut buf = [0u8; 256];
        let read = timeout(QUIET, stream.read(&mut buf)).await;

        // Silence, or the connection closed, but never console output.
        if let Ok(Ok(n)) = read {
            assert_eq!(
                n,
                0,
                "{cmd:?} answered: {:?}",
                String::from_utf8_lossy(&buf[..n])
            );
        }
    }
    let mut local = TcpClient::connect_from(LOCALHOST, server.nat_addr()).await;
    assert!(local.test_nat().await.port > 0);
    server.stop().await;
}
