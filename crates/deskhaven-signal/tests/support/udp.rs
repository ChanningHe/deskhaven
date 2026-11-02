//! UDP client: the controlled side's registration channel (RegisterPeer / RegisterPk) and
//! where hbbs delivers PunchHole / FetchLocalAddr / RequestRelay to it.

use super::{Conn, PeerKeys, msg, next_client_ip};
use hbb_common::{
    protobuf::Message as _,
    rendezvous_proto::{RegisterPeerResponse, RendezvousMessage, register_pk_response},
    tokio::{
        net::UdpSocket,
        time::{Duration, timeout},
    },
};
use std::net::{IpAddr, SocketAddr};

const MAX_DATAGRAM: usize = 65_536;

pub struct UdpClient {
    socket: UdpSocket,
    server: SocketAddr,
}

impl UdpClient {
    /// Binds a fresh loopback source address ([`next_client_ip`]).
    pub async fn bind(server: SocketAddr) -> Self {
        Self::bind_ip(next_client_ip(), server).await
    }

    pub async fn bind_ip(ip: IpAddr, server: SocketAddr) -> Self {
        let socket = UdpSocket::bind(SocketAddr::new(ip, 0)).await.unwrap();
        Self { socket, server }
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.socket.local_addr().unwrap()
    }

    pub async fn register_peer(&mut self, id: &str) -> RegisterPeerResponse {
        self.send(&msg::register_peer(id)).await;
        expect_msg!(self.recv().await, RegisterPeerResponse)
    }

    pub async fn register_pk(&mut self, id: &str, keys: &PeerKeys) -> register_pk_response::Result {
        self.send(&msg::register_pk(id, keys)).await;
        let res = expect_msg!(self.recv().await, RegisterPkResponse);
        res.result.enum_value().unwrap()
    }
}

impl Conn for UdpClient {
    async fn send(&mut self, msg: &RendezvousMessage) {
        let bytes = msg.write_to_bytes().unwrap();
        self.socket.send_to(&bytes, self.server).await.unwrap();
    }

    async fn try_recv(&mut self, wait: Duration) -> Option<RendezvousMessage> {
        let mut buf = vec![0u8; MAX_DATAGRAM];
        let (n, from) = timeout(wait, self.socket.recv_from(&mut buf))
            .await
            .ok()?
            .unwrap();
        assert_eq!(from, self.server, "datagram from a stranger");
        Some(RendezvousMessage::parse_from_bytes(&buf[..n]).unwrap())
    }
}
