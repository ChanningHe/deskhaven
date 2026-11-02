//! TCP client over hbb_common's `FramedStream` (the codec RustDesk clients use on 21116 /
//! 21115).

use super::{Conn, Rendezvous, STEP_MS, next_client_ip};
use hbb_common::{
    protobuf::Message as _, rendezvous_proto::RendezvousMessage, tcp::FramedStream,
    tokio::time::Duration,
};
use std::net::{IpAddr, SocketAddr};

pub struct TcpClient {
    stream: FramedStream,
}

impl TcpClient {
    /// Connects from a fresh loopback source address ([`next_client_ip`]).
    pub async fn connect(server: SocketAddr) -> Self {
        Self::connect_from(next_client_ip(), server).await
    }

    /// Connects from `ip`, e.g. the address of a UDP client to look like the same host.
    pub async fn connect_from(ip: IpAddr, server: SocketAddr) -> Self {
        let local = SocketAddr::new(ip, 0);
        let stream = FramedStream::new(server, Some(local), STEP_MS)
            .await
            .unwrap();
        Self { stream }
    }

    /// The address hbbs sees for this connection.
    pub fn local_addr(&self) -> SocketAddr {
        self.stream.local_addr()
    }

    /// The raw stream, for protocol steps the helpers do not cover (e.g. key exchange).
    pub fn stream(&mut self) -> &mut FramedStream {
        &mut self.stream
    }
}

impl Conn for TcpClient {
    async fn send(&mut self, msg: &RendezvousMessage) {
        self.stream.send(msg).await.unwrap();
    }

    async fn try_recv(&mut self, wait: Duration) -> Option<RendezvousMessage> {
        let ms = u64::try_from(wait.as_millis()).unwrap();
        let bytes = self.stream.next_timeout(ms).await?.ok()?;
        Some(RendezvousMessage::parse_from_bytes(&bytes).unwrap())
    }
}

impl Rendezvous for TcpClient {}
