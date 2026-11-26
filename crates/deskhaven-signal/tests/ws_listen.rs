// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use deskhaven_signal::SignalConfig;
use hbb_common::{
    rendezvous_proto::punch_hole_response::Failure::ID_NOT_EXIST,
    tokio::{self, net::TcpStream},
};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use support::{Conn, Rendezvous, TestServer, WsClient, addr, expect_msg};

const WS_IP: IpAddr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));

#[tokio::test]
async fn the_websocket_listener_binds_ws_listen_instead_of_port_plus_two() {
    let server = TestServer::start_with(|cfg| SignalConfig {
        ws_listen: Some(SocketAddr::new(WS_IP, cfg.port + 2)),
        ..cfg
    })
    .await;

    let mut ws = WsClient::connect(server.ws_addr()).await;
    ws.punch_hole_request(server.punch_req("no-such-peer"))
        .await;

    let res = expect_msg!(ws.recv().await, PunchHoleResponse);
    assert_eq!(res.failure.enum_value(), Ok(ID_NOT_EXIST));
    assert_eq!(server.ws_addr().ip(), WS_IP);
    assert!(TcpStream::connect(addr(server.port + 2)).await.is_err());
    server.stop().await;
}
