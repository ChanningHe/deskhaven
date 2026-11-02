//! UDP client: the controlled side's registration channel (RegisterPeer / RegisterPk) and
//! where hbbs delivers PunchHole / FetchLocalAddr / RequestRelay to it.

use super::{Conn, PeerKeys, Recv, msg, next_client_ip};
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

    /// Sends a datagram that need not be a valid protobuf.
    pub async fn send_raw(&mut self, bytes: &[u8]) {
        self.socket.send_to(bytes, self.server).await.unwrap();
    }
}

impl Conn for UdpClient {
    async fn send(&mut self, msg: &RendezvousMessage) {
        self.send_raw(&msg.write_to_bytes().unwrap()).await;
    }

    async fn recv_within(&mut self, wait: Duration) -> Recv {
        let mut buf = vec![0u8; MAX_DATAGRAM];
        let Ok(res) = timeout(wait, self.socket.recv_from(&mut buf)).await else {
            return Recv::Quiet;
        };
        let (n, from) = res.unwrap();
        assert_eq!(from, self.server, "datagram from a stranger");
        Recv::parse(&buf[..n])
    }
}
