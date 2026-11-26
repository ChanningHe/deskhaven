use hbb_common::{
    AddrMangle,
    bytes::Bytes,
    protobuf::Message as _,
    rendezvous_proto::{
        IdPk, LocalAddr, NatType, PunchHoleRequest, PunchHoleSent, RegisterPeer, RegisterPk,
        RelayResponse, RendezvousMessage, RequestRelay,
    },
};
use sodiumoxide::crypto::sign;
use std::net::SocketAddr;

#[derive(Clone, Debug)]
pub struct PeerKeys {
    pub uuid: Bytes,
    pub pk: Bytes,
}

impl PeerKeys {
    pub fn random() -> Self {
        Self {
            uuid: Bytes::copy_from_slice(uuid::Uuid::new_v4().as_bytes()),
            pk: Bytes::copy_from_slice(&sign::gen_keypair().0.0),
        }
    }

    pub fn with_new_pk(&self) -> Self {
        Self {
            pk: Self::random().pk,
            ..self.clone()
        }
    }
}

pub fn rv(set: impl FnOnce(&mut RendezvousMessage)) -> RendezvousMessage {
    let mut msg = RendezvousMessage::new();
    set(&mut msg);
    msg
}

pub fn register_peer(id: &str) -> RendezvousMessage {
    rv(|m| {
        m.set_register_peer(RegisterPeer {
            id: id.to_owned(),
            ..Default::default()
        })
    })
}

pub fn register_pk(id: &str, keys: &PeerKeys) -> RendezvousMessage {
    rv(|m| {
        m.set_register_pk(RegisterPk {
            id: id.to_owned(),
            uuid: keys.uuid.clone(),
            pk: keys.pk.clone(),
            ..Default::default()
        })
    })
}

/// Fields hbbs does not forward are set too, so tests can see that they are dropped.
pub fn punch_hole_request(target: &str, licence_key: &str) -> PunchHoleRequest {
    PunchHoleRequest {
        id: target.to_owned(),
        nat_type: NatType::ASYMMETRIC.into(),
        licence_key: licence_key.to_owned(),
        token: "controller-token".to_owned(),
        version: "1.4.0".to_owned(),
        udp_port: 40_000,
        upnp_port: 40_001,
        ..Default::default()
    }
}

pub fn request_relay(target: &str, licence_key: &str) -> RequestRelay {
    RequestRelay {
        id: target.to_owned(),
        uuid: "relay-session-uuid".to_owned(),
        relay_server: "192.0.2.50:21117".to_owned(),
        secure: true,
        licence_key: licence_key.to_owned(),
        token: "controller-token".to_owned(),
        ..Default::default()
    }
}

/// Mirrors the server's `MAX_RELAY_UUID_LEN`.
pub const MAX_RELAY_UUID_LEN: usize = 64;
/// Mirrors the server's `MAX_RELAY_SERVER_LEN`.
pub const MAX_RELAY_SERVER_LEN: usize = 259;

pub fn max_size_request_relay(target: &str, licence_key: &str) -> RequestRelay {
    RequestRelay {
        uuid: "u".repeat(MAX_RELAY_UUID_LEN),
        relay_server: format!("{}:21117", "r".repeat(MAX_RELAY_SERVER_LEN - 6)),
        ..request_relay(target, licence_key)
    }
}

pub fn decode_addr(mangled: &[u8]) -> SocketAddr {
    AddrMangle::decode(mangled)
}

pub fn encode_addr(addr: SocketAddr) -> Bytes {
    AddrMangle::encode(addr).into()
}

pub fn open_id_pk(signed: &[u8], server_pk: &sign::PublicKey) -> IdPk {
    let raw = sign::verify(signed, server_pk).unwrap();
    IdPk::parse_from_bytes(&raw).unwrap()
}

pub fn online_bits(states: &[u8], n: usize) -> Vec<bool> {
    (0..n)
        .map(|i| states[i / 8] & (0x80 >> (i % 8)) != 0)
        .collect()
}

/// Any non-empty value makes hbbs sign the target's pk into its answers.
pub const CLIENT_VERSION: &str = "1.4.0";

pub fn punch_hole_sent(controller: &Bytes, target: &str, relay_server: &str) -> PunchHoleSent {
    PunchHoleSent {
        socket_addr: controller.clone(),
        id: target.to_owned(),
        relay_server: relay_server.to_owned(),
        nat_type: NatType::SYMMETRIC.into(),
        version: CLIENT_VERSION.to_owned(),
        ..Default::default()
    }
}

pub fn local_addr(controller: &Bytes, target: &str, lan: SocketAddr) -> LocalAddr {
    LocalAddr {
        socket_addr: controller.clone(),
        local_addr: encode_addr(lan),
        relay_server: "192.0.2.60:21117".to_owned(),
        id: target.to_owned(),
        version: CLIENT_VERSION.to_owned(),
        ..Default::default()
    }
}

pub fn relay_response(forwarded: &RequestRelay, target: &str) -> RelayResponse {
    let mut res = RelayResponse {
        socket_addr: forwarded.socket_addr.clone(),
        uuid: forwarded.uuid.clone(),
        relay_server: forwarded.relay_server.clone(),
        version: CLIENT_VERSION.to_owned(),
        ..Default::default()
    };
    res.set_id(target.to_owned());
    res
}
