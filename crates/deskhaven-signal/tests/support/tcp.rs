//! TCP client over hbb_common's `FramedStream` (the codec RustDesk clients use on 21116 /
//! 21115).

use super::{Conn, Recv, Rendezvous, STEP_MS, next_client_ip};
use hbb_common::{
    rendezvous_proto::{KeyExchange, RendezvousMessage, rendezvous_message},
    tcp::FramedStream,
    tokio::time::{Duration, Instant, timeout_at},
};
use std::net::{IpAddr, SocketAddr};

/// Server-first KeyExchange frames a RustDesk client skips at most (client `secure_tcp`).
const MAX_SKIPPED_KX: usize = 2;

pub struct TcpClient {
    stream: FramedStream,
    /// KeyExchange frames that arrived before any other message.
    key_exchanges: Vec<KeyExchange>,
    /// A non-KeyExchange message arrived: later KeyExchange frames are ordinary messages.
    past_handshake: bool,
}

impl TcpClient {
    /// Connects from a fresh loopback source address ([`next_client_ip`]).
    pub async fn connect(server: SocketAddr) -> Self {
        Self::connect_from(next_client_ip(), server).await
    }

    /// Connects from `ip`, e.g. the address of a UDP client to look like the same host.
    pub async fn connect_from(ip: IpAddr, server: SocketAddr) -> Self {
        Self::from_stream(Self::connect_raw(ip, server).await)
    }

    /// A bare connection, for protocol steps the client does not model (e.g. answering a
    /// key exchange); wrap it with [`TcpClient::from_stream`] afterwards.
    pub async fn connect_raw(ip: IpAddr, server: SocketAddr) -> FramedStream {
        let local = SocketAddr::new(ip, 0);
        FramedStream::new(server, Some(local), STEP_MS)
            .await
            .unwrap()
    }

    pub fn from_stream(stream: FramedStream) -> Self {
        Self {
            stream,
            key_exchanges: Vec::new(),
            past_handshake: false,
        }
    }

    /// The address hbbs sees for this connection.
    pub fn local_addr(&self) -> SocketAddr {
        self.stream.local_addr()
    }

    /// The raw stream (e.g. to install a cipher with `set_key` after a key exchange).
    pub fn stream(&mut self) -> &mut FramedStream {
        &mut self.stream
    }

    pub fn into_stream(self) -> FramedStream {
        self.stream
    }

    /// KeyExchange frames the server sent first. Like a RustDesk client, receiving skips up to
    /// two of them, so `recv` / `expect_msg!` see the first real answer. Empty while the server
    /// sends no KeyExchange (upstream behavior until M1-07).
    pub fn key_exchanges(&self) -> &[KeyExchange] {
        &self.key_exchanges
    }

    async fn next_frame(&mut self, deadline: Instant) -> Recv {
        match timeout_at(deadline, self.stream.next()).await {
            Err(_) => Recv::Quiet,
            Ok(Some(Ok(bytes))) => Recv::parse(&bytes),
            Ok(Some(Err(_)) | None) => Recv::Closed,
        }
    }

    /// Captures `msg` if it is a skippable server-first KeyExchange.
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
                Recv::Msg(msg) if self.capture_kx(&msg) => {}
                other => return other,
            }
        }
    }
}

impl Rendezvous for TcpClient {}
