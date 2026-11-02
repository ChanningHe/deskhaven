//! Upstream-behavior baseline (M1-04): malformed input and the removed loopback console.
//!
//! Pins upstream hbbs at 873230e (`rs` = src/rendezvous_server.rs at that commit): every
//! handler starts with `RendezvousMessage::parse_from_bytes` and ignores what does not parse
//! (rs:372 UDP, rs:531 TCP / websocket, rs:1154 NAT port). After each malformed input the
//! server must still answer a valid request.

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

/// rs:372-373: datagrams that do not parse (garbage, a truncated protobuf) get no answer and do
/// not disturb the UDP loop.
#[tokio::test]
async fn udp_garbage_and_truncated_protobuf_get_no_answer() {
    // Arrange
    let server = TestServer::start().await;
    let mut client = server.udp().await;

    // Act
    client.send_raw(&GARBAGE).await;
    client.send_raw(&truncated_protobuf()).await;

    // Assert
    client.assert_silent().await;
    assert!(client.register_peer("still-serving").await.request_pk);
    server.stop().await;
}

/// rs:531 (main port) and rs:1153-1154 (NAT port): a well-framed first frame that is not a
/// protobuf gets no answer and the connection is closed; new connections are served.
#[tokio::test]
async fn tcp_garbage_first_frame_closes_the_connection() {
    // Arrange
    let server = TestServer::start().await;

    for addr in [server.main_addr(), server.nat_addr()] {
        for payload in [GARBAGE.to_vec(), truncated_protobuf()] {
            let mut client = TcpClient::connect(addr).await;

            // Act
            client.stream().send_raw(payload).await.unwrap();

            // Assert
            client.assert_closed().await;
        }
        assert!(
            TcpClient::connect(addr).await.test_nat().await.port > 0,
            "{addr}"
        );
    }
    server.stop().await;
}

/// rs:1221-1227: websocket text frames are skipped (the connection stays usable), while a
/// binary frame that is not a protobuf ends the connection.
#[tokio::test]
async fn websocket_text_frame_is_skipped_and_garbage_binary_closes() {
    // Arrange
    let server = TestServer::start().await;
    let mut texting = server.ws().await;
    let mut garbling = server.ws().await;

    // Act
    texting.send_frame(Message::Text("hello".into())).await;
    garbling
        .send_frame(Message::Binary(GARBAGE.to_vec().into()))
        .await;

    // Assert
    assert!(
        texting.test_nat().await.port > 0,
        "same connection still answers"
    );
    garbling.assert_closed().await;
    assert!(server.ws().await.test_nat().await.port > 0);
    server.stop().await;
}

/// Upstream rs:1136-1148 handed loopback connections to the NAT port to the unauthenticated
/// text console (`h` help, `rs` set relays, ...); removed in M1-03 (CHANGES.md). Console
/// commands from 127.0.0.1 get no output, and loopback clients get normal NAT-port answers.
#[tokio::test]
async fn loopback_console_commands_on_nat_port_get_no_output() {
    // Arrange
    let server = TestServer::start().await;

    for cmd in ["h\n", "rs 192.0.2.66\n"] {
        let mut stream = TcpStream::connect(server.nat_addr()).await.unwrap();
        assert!(stream.local_addr().unwrap().ip().is_loopback());

        // Act
        stream.write_all(cmd.as_bytes()).await.unwrap();
        let mut buf = [0u8; 256];
        let read = timeout(QUIET, stream.read(&mut buf)).await;

        // Assert: silence, or the connection closed, but never console output.
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
