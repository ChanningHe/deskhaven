//! Shared protocol test harness (M1-04) for the deskhaven-signal integration tests.
//!
//! - [`TestServer`]: a `SignalServer` on free loopback ports, with its addresses, licence key,
//!   public key and handle; [`TestServer::stop`] checks the clean shutdown.
//! - Clients speaking the wire protocol the way RustDesk clients and hbbs do (hbb_common
//!   protobufs): [`UdpClient`] (one protobuf per datagram, what hbbs' `FramedSocket` sends),
//!   [`TcpClient`] (hbb_common `FramedStream`, i.e. `BytesCodec` length framing) and
//!   [`WsClient`] (one protobuf per binary websocket message). [`Conn`] is the common
//!   send / receive surface, [`Rendezvous`] the typed request helpers of the stream transports.
//!
//! Determinism: the server keeps process-wide state (`IP_BLOCKER`: RegisterPk per source IP;
//! `ROTATION_RELAY_SERVER`: the relay round-robin cursor). Every client binds its own loopback
//! source address from [`next_client_ip`] so no two clients share an `IP_BLOCKER` entry, and
//! tests never assert the rotation order. This relies on Linux routing all of 127.0.0.0/8 to
//! `lo` (macOS only configures 127.0.0.1 by default).

// Each test binary uses a different subset of the harness.
#![allow(dead_code, unused_imports)]

/// The payload of the `$variant` arm of a `RendezvousMessage`; panics with the actual message
/// otherwise.
macro_rules! expect_msg {
    ($msg:expr, $variant:ident) => {
        match $msg.union {
            Some(hbb_common::rendezvous_proto::rendezvous_message::Union::$variant(inner)) => inner,
            other => panic!("expected {}, got {other:?}", stringify!($variant)),
        }
    };
}
pub(crate) use expect_msg;

mod conn;
mod msg;
mod net;
mod server;
mod tcp;
mod udp;
mod ws;

pub use conn::{Conn, Rendezvous};
pub use msg::*;
pub use net::*;
pub use server::{TestServer, config, wait_until_ready};
pub use tcp::TcpClient;
pub use udp::UdpClient;
pub use ws::WsClient;
