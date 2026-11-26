// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use hbb_common::{rendezvous_proto::OnlineRequest, tokio};
use support::{Conn, Rendezvous, TcpClient, TestServer, online_bits, rv};

#[tokio::test]
async fn test_nat_reports_client_source_port_on_nat_and_main_ports() {
    let server = TestServer::start().await;

    for addr in [server.nat_addr(), server.main_addr()] {
        let mut client = TcpClient::connect(addr).await;

        let res = client.test_nat().await;

        assert_eq!(res.port, i32::from(client.local_addr().port()), "{addr}");
    }
    server.stop().await;
}

#[tokio::test]
async fn online_request_reports_one_bit_per_id_msb_first() {
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

    let res = server.nat().await.online_request(&ids).await;

    assert_eq!(res.states.as_ref(), [0b1000_0000, 0b1000_0000]);
    let bits = online_bits(&res.states, ids.len());
    let online: Vec<_> = ids.iter().zip(bits).filter(|(_, on)| *on).collect();
    assert_eq!(online, [(&"online-peer-a", true), (&"online-peer-b", true)]);
    server.stop().await;
}

#[tokio::test]
async fn online_request_on_main_port_gets_no_answer() {
    let server = TestServer::start().await;
    let (_peer, _) = server.online_peer("online-peer-a").await;
    let mut client = server.tcp().await;
    let req = OnlineRequest {
        id: "asker".to_owned(),
        peers: vec!["online-peer-a".to_owned()],
        ..Default::default()
    };

    client.send(&rv(|m| m.set_online_request(req))).await;

    client.assert_closed().await;
    server.stop().await;
}
