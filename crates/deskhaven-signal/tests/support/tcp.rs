use super::{Conn, Recv, Rendezvous, STEP_MS, io, next_client_ip};
use hbb_common::{
    bytes::Bytes,
    rendezvous_proto::{KeyExchange, RendezvousMessage, rendezvous_message},
    tcp::FramedStream,
    tokio::{
        net::TcpSocket,
        time::{Duration, Instant, timeout_at},
    },
};
use std::net::{IpAddr, SocketAddr};

/// A RustDesk client skips at most this many server-first KeyExchange frames.
const MAX_SKIPPED_KX: usize = 2;
/// Requested SO_RCVBUF of [`TcpClient::connect_small_buffer`] (the kernel rounds it up to its minimum).
const SMALL_RECV_BUFFER: u32 = 1;

pub struct TcpClient {
    stream: FramedStream,
    key_exchanges: Vec<KeyExchange>,
    /// A non-KeyExchange message arrived: later KeyExchange frames are ordinary messages.
    past_handshake: bool,
    heartbeats: usize,
}

impl TcpClient {
    pub async fn connect(server: SocketAddr) -> Self {
        Self::connect_from(next_client_ip(), server).await
    }

    pub async fn connect_from(ip: IpAddr, server: SocketAddr) -> Self {
        Self::from_stream(Self::connect_raw(ip, server).await)
    }

    /// The smallest receive buffer the kernel allows, so the server's sends block soon after
    /// the client stops reading.
    pub async fn connect_small_buffer(server: SocketAddr) -> Self {
        let socket = TcpSocket::new_v4().unwrap();
        socket.set_recv_buffer_size(SMALL_RECV_BUFFER).unwrap();
        socket.bind(SocketAddr::new(next_client_ip(), 0)).unwrap();
        let stream = socket.connect(server).await.unwrap();
        let local = stream.local_addr().unwrap();
        Self::from_stream(FramedStream::from(stream, local))
    }

    pub async fn connect_raw(ip: IpAddr, server: SocketAddr) -> FramedStream {
        let local = SocketAddr::new(ip, 0);
        io(
            "tcp connect",
            FramedStream::new(server, Some(local), STEP_MS),
        )
        .await
        .unwrap()
    }

    pub fn from_stream(stream: FramedStream) -> Self {
        Self {
            stream,
            key_exchanges: Vec::new(),
            past_handshake: false,
            heartbeats: 0,
        }
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.stream.local_addr()
    }

    pub fn stream(&mut self) -> &mut FramedStream {
        &mut self.stream
    }

    pub fn into_stream(self) -> FramedStream {
        self.stream
    }

    pub fn key_exchanges(&self) -> &[KeyExchange] {
        &self.key_exchanges
    }

    pub fn heartbeats(&self) -> usize {
        self.heartbeats
    }

    /// `None` for a server heartbeat, answered with an empty frame like a RustDesk client in TCP
    /// mode.
    async fn next_frame(&mut self, deadline: Instant) -> Option<Recv> {
        match timeout_at(deadline, self.stream.next()).await {
            Err(_) => Some(Recv::Quiet),
            Ok(Some(Ok(bytes))) if bytes.is_empty() => {
                self.heartbeats += 1;
                self.stream.send_bytes(Bytes::new()).await.ok();
                None
            }
            Ok(Some(Ok(bytes))) => Some(Recv::parse(&bytes)),
            Ok(Some(Err(_)) | None) => Some(Recv::Closed),
        }
    }

    fn capture_kx(&mut self, msg: &RendezvousMessage) -> bool {
        let skippable = !self.past_handshake && self.key_exchanges.len() < MAX_SKIPPED_KX;
        match &msg.union {
            Some(rendezvous_message::Union::KeyExchange(kx)) if skippable => {
                self.key_exchanges.push(kx.clone());
                true
            }
            _ => {
                self.past_handshake = true;
                false
            }
        }
    }
}

impl Conn for TcpClient {
    async fn send(&mut self, msg: &RendezvousMessage) {
        self.stream.send(msg).await.unwrap();
    }

    async fn recv_within(&mut self, wait: Duration) -> Recv {
        let deadline = Instant::now() + wait;
        loop {
            match self.next_frame(deadline).await {
                None => {}
                Some(Recv::Msg(msg)) if self.capture_kx(&msg) => {}
                Some(other) => return other,
            }
        }
    }
}

impl Rendezvous for TcpClient {}
