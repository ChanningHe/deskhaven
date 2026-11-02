//! Shared protocol test harness (M1-04) for the deskhaven-signal integration tests.
//!
//! - [`TestServer`]: a `SignalServer` on free loopback ports, with its addresses, licence key,
//!   public key and handle; [`TestServer::stop`] checks the clean shutdown,
//!   [`TestServer::restart`] brings the same server (ports, key) back.
//! - Clients speaking the wire protocol the way RustDesk clients and hbbs do (hbb_common
//!   protobufs): [`UdpClient`] (one protobuf per datagram, what hbbs' `FramedSocket` sends),
//!   [`TcpClient`] (hbb_common `FramedStream`, i.e. `BytesCodec` length framing; skips and
//!   records server-first KeyExchange frames like a RustDesk client) and [`WsClient`] (one
//!   protobuf per binary websocket message). [`Conn`] is the common send / receive surface
//!   ([`Recv`] tells silence from a closed connection), [`Rendezvous`] the typed request
//!   helpers of the stream transports; `flows` holds multi-step flows.
//! - `kx` (M1-07): the client half of the KeyExchange as RustDesk clients run it (both client
//!   generations), to secure a [`TcpClient`] and to check the server's offer.
//!
//! Determinism: the server keeps process-wide state (`IP_BLOCKER`: RegisterPk per source IP).
//! Every client binds its own loopback source address from [`next_client_ip`] so no two
//! clients share an `IP_BLOCKER` entry. The relay round-robin cursor lives in each server's
//! `AllowAll` (it was process-wide upstream); only the `relay_rotation` binary asserts the
//! rotation order. Requires Linux routing all of 127.0.0.0/8 to `lo` (macOS only configures
//! 127.0.0.1): the first `next_client_ip` call probes 127.77.0.1 and fails with that
//! explanation otherwise.

// Each test binary uses a different subset of the harness.
#![allow(dead_code)]

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
#[allow(unused_imports)] // re-exports: each test binary uses a subset
pub(crate) use expect_msg;

mod clock;
mod conn;
mod flows;
mod kx;
mod msg;
mod net;
mod server;
mod tcp;
mod udp;
mod ws;

#[allow(unused_imports)]
pub use {
    clock::{IO_DEADLINE, io},
    conn::{Conn, Recv, Rendezvous},
    flows::*,
    kx::{
        KxClient, SIGNED_PARAMS_FLAG, answer, read_offer, reply, secure, secured_tcp, verify_offer,
    },
    msg::*,
    net::*,
    server::{TestServer, TrustXff, allow_all, config, memory_store, trusting, wait_until_ready},
    tcp::TcpClient,
    udp::UdpClient,
    ws::WsClient,
};
