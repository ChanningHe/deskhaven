//! Websocket client (main port + 2): one protobuf per binary message, like RustDesk's
//! websocket transport.

use super::{Conn, Recv, Rendezvous, io, next_client_ip};
use hbb_common::{
    futures::{SinkExt, StreamExt},
    protobuf::Message as _,
    rendezvous_proto::RendezvousMessage,
    tokio::{
        net::{TcpSocket, TcpStream},
        time::{Duration, Instant, timeout_at},
    },
};
use std::net::{IpAddr, SocketAddr};
use tokio_tungstenite::{
    WebSocketStream, client_async,
    tungstenite::{
        Message,
        client::IntoClientRequest,
        http::{HeaderName, HeaderValue},
    },
};

pub struct WsClient {
    ws: WebSocketStream<TcpStream>,
    local: SocketAddr,
}

impl WsClient {
    /// Connects from a fresh loopback source address ([`next_client_ip`]).
    pub async fn connect(server: SocketAddr) -> Self {
        Self::connect_from(next_client_ip(), server, &[]).await
    }

    /// Connects from `ip` with extra handshake headers (e.g. what a reverse proxy adds);
    /// connect + handshake must finish within [`super::IO_DEADLINE`] of real time.
    pub async fn connect_from(ip: IpAddr, server: SocketAddr, headers: &[(&str, &str)]) -> Self {
        io(
            "websocket connect + handshake",
            Self::handshake(ip, server, headers),
        )
        .await
    }

    async fn handshake(ip: IpAddr, server: SocketAddr, headers: &[(&str, &str)]) -> Self {
        let socket = TcpSocket::new_v4().unwrap();
        socket.bind(SocketAddr::new(ip, 0)).unwrap();
        let stream = socket.connect(server).await.unwrap();
        let local = stream.local_addr().unwrap();
        let mut req = format!("ws://{server}/").into_client_request().unwrap();
        for (name, value) in headers {
            req.headers_mut().insert(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        let (ws, _) = client_async(req, stream).await.unwrap();
        Self { ws, local }
    }

    /// The TCP address of this connection.
    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }

    /// Sends any frame, e.g. a text frame or a binary frame that is not a protobuf.
    pub async fn send_frame(&mut self, frame: Message) {
        self.ws.send(frame).await.unwrap();
    }
}

impl Conn for WsClient {
    async fn send(&mut self, msg: &RendezvousMessage) {
        let bytes = msg.write_to_bytes().unwrap();
        self.send_frame(Message::Binary(bytes.into())).await;
    }

    /// Skips control frames; answers server heartbeats (empty binary messages) with an empty
    /// one, like a RustDesk client in websocket mode.
    async fn recv_within(&mut self, wait: Duration) -> Recv {
        let deadline = Instant::now() + wait;
        loop {
            match timeout_at(deadline, self.ws.next()).await {
                Err(_) => return Recv::Quiet,
                Ok(Some(Ok(Message::Binary(bytes)))) if bytes.is_empty() => {
                    self.ws.send(Message::Binary(bytes)).await.ok();
                }
                Ok(Some(Ok(Message::Binary(bytes)))) => {
                    return Recv::parse(&bytes);
                }
                Ok(Some(Ok(Message::Close(_)) | Err(_)) | None) => return Recv::Closed,
                Ok(Some(Ok(_))) => {}
            }
        }
    }
}

impl Rendezvous for WsClient {}
