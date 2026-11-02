//! Upstream-behavior baseline (M1-04): NAT test and online queries.
//!
//! Pins upstream hbbs at 873230e (`rs` = src/rendezvous_server.rs at that commit). The NAT
//! test port (main port - 1, `handle_listener2` rs:1134-1172) answers one TestNatRequest or
//! OnlineRequest per connection.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use hbb_common::{rendezvous_proto::OnlineRequest, tokio};
use support::{Conn, Rendezvous, TcpClient, TestServer, online_bits, rv};

/// rs:1156-1162 (NAT port) and rs:581-595 (main port): TestNatResponse.port is the client's
/// source port as the server sees it.
#[tokio::test]
async fn test_nat_reports_client_source_port_on_nat_and_main_ports() {
    // Arrange
    let server = TestServer::start().await;

    for addr in [server.nat_addr(), server.main_addr()] {
        let mut client = TcpClient::connect(addr).await;

        // Act
        let res = client.test_nat().await;

        // Assert
        assert_eq!(res.port, i32::from(client.local_addr().port()), "{addr}");
    }
    server.stop().await;
}

/// rs:1164-1166, 825-851: OnlineRequest on the NAT port answers one bit per requested id,
/// most significant bit first, set when the peer is in memory and was seen within
/// REG_TIMEOUT (30 s); an id that only sent RegisterPeer is not stored (rs:641-643).
#[tokio::test]
async fn online_request_reports_one_bit_per_id_msb_first() {
    // Arrange
    let server = TestServer::start().await;
    let (_a, _) = server.online_peer("online-peer-a").await;
    let (_b, _) = server.online_peer("online-peer-b").await;
    let mut pk_less = server.udp().await;
    assert!(pk_less.register_peer("peer-without-pk").await.request_pk);
    let ids = [
        "online-peer-a",
        "unknown-1",
        "peer-without-pk",
        "unknown-3",
        "unknown-4",
        "unknown-5",
        "unknown-6",
        "unknown-7",
        "online-peer-b",
        "unknown-9",
    ];

    // Act
    let res = server.nat().await.online_request(&ids).await;

    // Assert
    assert_eq!(res.states.as_ref(), [0b1000_0000, 0b1000_0000]);
    let bits = online_bits(&res.states, ids.len());
    let online: Vec<_> = ids.iter().zip(bits).filter(|(_, on)| *on).collect();
    assert_eq!(online, [(&"online-peer-a", true), (&"online-peer-b", true)]);
    server.stop().await;
}

/// rs:523-609 has no OnlineRequest arm: on the main port it gets no answer (the connection is
/// closed).
#[tokio::test]
async fn online_request_on_main_port_gets_no_answer() {
    // Arrange
    let server = TestServer::start().await;
    let (_peer, _) = server.online_peer("online-peer-a").await;
    let mut client = server.tcp().await;
    let req = OnlineRequest {
        id: "asker".to_owned(),
        peers: vec!["online-peer-a".to_owned()],
        ..Default::default()
    };

    // Act
    client.send(&rv(|m| m.set_online_request(req))).await;

    // Assert
    client.assert_closed().await;
    server.stop().await;
}
