//! Fuzzing entry points (deskhaven code, feature `fuzzing`, M4-05): untrusted bytes through the
//! same handlers the server runs, without sockets for the stream side.
//!
//! - [`Harness::udp`]: one datagram through `handle_udp` (replies go to a local socket nobody
//!   reads);
//! - [`Harness::tcp`]: the frames of one main-port connection: the KeyExchange state machine
//!   (an offer is made first, as at accept), then `handle_tcp` for every message frame, then
//!   the end-of-connection cleanup. There is no sink, so answers are dropped.
//!
//! The harness keeps its server across calls (state-dependent paths are reachable) and can be
//! [`Harness::reset`] to bound memory.

use super::*;

/// A rendezvous server without listeners.
pub struct Harness {
    rs: RendezvousServer,
    rx: Receiver,
    socket: FramedSocket,
    /// Source of every datagram and connection: a socket that only collects replies.
    client: std::net::UdpSocket,
    key: String,
    stop: tokio_util::sync::CancellationToken,
}

impl Harness {
    /// A fresh server on loopback. Must run inside a tokio runtime.
    pub async fn new() -> ResultType<Self> {
        let cfg = SignalConfig {
            punch_req_max: 0,
            ..SignalConfig::new(sign::gen_keypair().1)
        };
        let (tx, rx) = mpsc::unbounded_channel();
        let stop = tokio_util::sync::CancellationToken::new();
        let tasks = Tasks::new(stop.clone());
        let store = Arc::new(crate::store::MemoryPeerStore::new());
        let control = Arc::new(crate::AllowAll::new());
        let switches = crate::server::Switches::new(&cfg);
        let rs = RendezvousServer::new(&cfg, store, control, tx, switches, tasks).await?;
        let socket = create_udp_listener(Some(IpAddr::V4(Ipv4Addr::LOCALHOST)), 0, 0).await?;
        let client = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))?;
        client.set_nonblocking(true)?;
        Ok(Self {
            rs,
            rx,
            socket,
            client,
            key: cfg.licence_key(),
            stop,
        })
    }

    /// Replaces the server with a fresh one (its tasks are stopped).
    pub async fn reset(&mut self) -> ResultType<()> {
        *self = Self::new().await?;
        Ok(())
    }

    /// The licence key clients of this server must send.
    pub fn licence_key(&self) -> &str {
        &self.key
    }

    fn source(&self) -> SocketAddr {
        self.client
            .local_addr()
            .unwrap_or_else(|_| SocketAddr::from((Ipv4Addr::LOCALHOST, 9)))
    }

    /// One datagram on the UDP port.
    pub async fn udp(&mut self, datagram: &[u8]) {
        let from = self.source();
        let key = self.key.clone();
        let bytes = BytesMut::from(datagram);
        let _ = self
            .rs
            .handle_udp(&bytes, from, &mut self.socket, &key)
            .await;
        self.drain();
    }

    /// One main-port connection that sends `frames`, in order.
    pub async fn tcp<'a>(&mut self, frames: impl IntoIterator<Item = &'a [u8]>) {
        let addr = self.source();
        let key = self.key.clone();
        let Ok((mut kx, _offer)) = KxState::offer(&self.rs.inner.sk) else {
            return;
        };
        let mut sink = None;
        let mut conn = StreamPeer::default();
        for frame in frames {
            let bytes = match kx.on_frame(Bytes::copy_from_slice(frame), false) {
                Ok(kx::Frame::Message(bytes)) => bytes,
                Ok(kx::Frame::Secured(_)) => {
                    conn.secured = true;
                    continue;
                }
                Err(_) => break,
            };
            if bytes.is_empty()
                && let Some(id) = &conn.id
            {
                self.rs.touch_stream_peer(id, addr).await;
            } else if !self
                .rs
                .handle_tcp(&bytes, &mut sink, addr, &key, false, &mut conn)
                .await
            {
                break;
            }
            self.drain();
        }
        if let Some(id) = &conn.id {
            self.rs.unbind_stream_peer(id, addr).await;
        }
        self.drain();
    }

    /// Runs what the handlers queued for the server loop, as `io_loop` does.
    fn drain(&mut self) {
        while self.rx.try_recv().is_ok() {}
        let mut buf = [0u8; 2048];
        while self.client.recv(&mut buf).is_ok() {}
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    fn frame(msg: &RendezvousMessage) -> Vec<u8> {
        msg.write_to_bytes().unwrap()
    }

    #[hbb_common::tokio::test]
    async fn the_harness_registers_over_udp_and_serves_tcp_frames() {
        let mut h = Harness::new().await.unwrap();
        let mut reg = RendezvousMessage::new();
        reg.set_register_pk(RegisterPk {
            id: "fuzz-peer-1".to_owned(),
            uuid: Bytes::from_static(b"0123456789abcdef"),
            pk: Bytes::from(vec![7; REGISTER_PK_LEN]),
            ..Default::default()
        });
        h.udp(&frame(&reg)).await;
        assert!(h.rs.pm.get_in_memory("fuzz-peer-1").await.is_some());

        let mut punch = RendezvousMessage::new();
        punch.set_punch_hole_request(PunchHoleRequest {
            id: "fuzz-peer-1".to_owned(),
            licence_key: h.licence_key().to_owned(),
            ..Default::default()
        });
        let frames = [frame(&punch), Vec::new(), b"\xff\x00garbage".to_vec()];
        h.tcp(frames.iter().map(Vec::as_slice)).await;
        h.reset().await.unwrap();
        assert!(h.rs.pm.get_in_memory("fuzz-peer-1").await.is_none());
    }
}
