//! A device's registration channel: `RegisterPeer` / `RegisterPk` over UDP with the
//! client's protobufs (`hbb_common::rendezvous_proto`, `src/rendezvous_mediator.rs`).

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use hbb_common::bytes::Bytes;
use hbb_common::protobuf::Message as _;
use hbb_common::rendezvous_proto::{
    RegisterPeer, RegisterPk, RendezvousMessage, register_pk_response, rendezvous_message::Union,
};
use tokio::net::UdpSocket;
use tokio::time::{Instant, timeout_at};

use crate::stats::Failure;

/// Rendezvous answers are a few dozen bytes.
const MAX_DATAGRAM: usize = 2048;

/// What a device registers (`Config::get_key_pair`, `hbb_common::get_uuid`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keys {
    pub id: String,
    pub uuid: Vec<u8>,
    pub pk: Vec<u8>,
}

pub struct Rendezvous {
    socket: UdpSocket,
    server: SocketAddr,
}

impl Rendezvous {
    pub async fn bind(local: IpAddr, server: SocketAddr) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(SocketAddr::new(local, 0)).await?;
        Ok(Self { socket, server })
    }

    /// `RegisterPk` (`rendezvous_mediator.rs:1255-1283`); `Ok` only for result `OK`.
    pub async fn register_pk(&self, keys: &Keys, wait: Duration) -> Result<(), Failure> {
        let mut msg = RendezvousMessage::new();
        msg.set_register_pk(RegisterPk {
            id: keys.id.clone(),
            uuid: Bytes::copy_from_slice(&keys.uuid),
            pk: Bytes::copy_from_slice(&keys.pk),
            ..Default::default()
        });
        match self.exchange(&msg, wait).await? {
            Union::RegisterPkResponse(res) => match res.result.enum_value() {
                Ok(register_pk_response::Result::OK) => Ok(()),
                other => Err(Failure::Body(format!("RegisterPkResponse {other:?}"))),
            },
            other => Err(unexpected(&other)),
        }
    }

    /// `RegisterPeer` (`rendezvous_mediator.rs:1302-1330`); `Ok(true)` when the server asks
    /// for the key (`request_pk`).
    pub async fn register_peer(&self, id: &str, wait: Duration) -> Result<bool, Failure> {
        let mut msg = RendezvousMessage::new();
        msg.set_register_peer(RegisterPeer {
            id: id.to_owned(),
            ..Default::default()
        });
        match self.exchange(&msg, wait).await? {
            Union::RegisterPeerResponse(res) => Ok(res.request_pk),
            other => Err(unexpected(&other)),
        }
    }

    /// Sends `msg` and returns the first message the server sends back within `wait`.
    async fn exchange(&self, msg: &RendezvousMessage, wait: Duration) -> Result<Union, Failure> {
        let deadline = Instant::now() + wait;
        let bytes = msg
            .write_to_bytes()
            .map_err(|err| Failure::Transport(format!("encode: {err}")))?;
        self.socket
            .send_to(&bytes, self.server)
            .await
            .map_err(|err| Failure::Transport(format!("send: {err}")))?;
        let mut buf = [0u8; MAX_DATAGRAM];
        loop {
            let (n, from) = timeout_at(deadline, self.socket.recv_from(&mut buf))
                .await
                .map_err(|_| Failure::Timeout)?
                .map_err(|err| Failure::Transport(format!("recv: {err}")))?;
            if from != self.server {
                continue;
            }
            let parsed = RendezvousMessage::parse_from_bytes(&buf[..n])
                .map_err(|err| Failure::Body(format!("decode: {err}")))?;
            if let Some(union) = parsed.union {
                return Ok(union);
            }
        }
    }
}

fn unexpected(union: &Union) -> Failure {
    let name = format!("{union:?}");
    let name = name.split(['(', ' ']).next().unwrap_or_default();
    Failure::Body(format!("unexpected {name}"))
}
