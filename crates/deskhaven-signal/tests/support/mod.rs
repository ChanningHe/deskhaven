//! Every client binds its own loopback source address from [`next_client_ip`], so no two
//! clients share a per-IP limit entry. Requires Linux routing all of 127.0.0.0/8 to `lo`.

// Each test binary uses a different subset of the harness.
#![allow(dead_code)]

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
mod web;
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
    web::*,
    ws::WsClient,
};
