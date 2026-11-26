//! The client half of the rendezvous KeyExchange, mirroring RustDesk's.

use super::{STEP_MS, TcpClient, io};
use hbb_common::{
    bytes::Bytes,
    protobuf::Message as _,
    rendezvous_proto::{KeyExchange, KxParams, RendezvousMessage, rendezvous_message},
    tcp::{KX_PARAMS_DOMAIN, KxTranscript, kx_version_for},
};
use sodiumoxide::crypto::{box_, secretbox, sign};

/// Bit 255 of the signed X25519 key: the server signs its parameters (`signed_params`).
pub const SIGNED_PARAMS_FLAG: u8 = 0x80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KxClient {
    /// Before Kx v1 (e.g. 1.4.1): checks only the signature over `keys[0]`, replies without a
    /// version (0) and uses one key for both directions.
    Legacy,
    /// Current: also checks the signed parameters announced by the key's high bit or a
    /// non-empty `signed_params`, picks `min(advertised, 1)` and splits the key for v1.
    Current,
}

pub fn verify_offer(
    ex: &KeyExchange,
    rs_pk: &sign::PublicKey,
    client: KxClient,
) -> Result<[u8; 32], String> {
    if ex.keys.len() != 1 {
        return Err("Handshake failed: invalid key exchange message".to_owned());
    }
    let pk = sign::verify(&ex.keys[0], rs_pk)
        .map_err(|()| "Signature mismatch in key exchange".to_owned())?;
    let pk: [u8; 32] = pk
        .try_into()
        .map_err(|_| "Wrong their public length in key exchange".to_owned())?;
    let announced = pk[31] & SIGNED_PARAMS_FLAG != 0 || !ex.signed_params.is_empty();
    if client == KxClient::Current && announced {
        let params = sign::verify(&ex.signed_params, rs_pk)
            .ok()
            .and_then(|signed| {
                let params = signed.strip_prefix(KX_PARAMS_DOMAIN)?;
                KxParams::parse_from_bytes(params).ok()
            })
            .ok_or_else(|| "Missing or invalid signed key exchange parameters".to_owned())?;
        if params.pk[..] != pk[..] || params.version != ex.version {
            return Err("Key exchange version or public key does not match its signature".into());
        }
    }
    Ok(pk)
}

fn picked_by(client: KxClient, advertised: u32) -> u32 {
    match client {
        KxClient::Legacy => 0,
        KxClient::Current => kx_version_for(advertised),
    }
}

pub struct Reply {
    pub msg: RendezvousMessage,
    pub our_pk: Bytes,
    pub key: secretbox::Key,
}

pub fn reply(their_pk: [u8; 32], picked: u32) -> Reply {
    let (our_pk, our_sk) = box_::gen_keypair();
    let key = secretbox::gen_key();
    let nonce = box_::Nonce([0u8; box_::NONCEBYTES]);
    let sealed = box_::seal(&key.0, &nonce, &box_::PublicKey(their_pk), &our_sk);
    let our_pk = Bytes::copy_from_slice(&our_pk.0);
    let mut msg = RendezvousMessage::new();
    msg.set_key_exchange(KeyExchange {
        keys: vec![our_pk.clone(), sealed.into()],
        version: picked,
        ..Default::default()
    });
    Reply { msg, our_pk, key }
}

pub async fn read_offer(client: &mut TcpClient) -> KeyExchange {
    let bytes = io(
        "the KeyExchange offer",
        client.stream().next_timeout(STEP_MS),
    )
    .await
    .expect("a first frame")
    .expect("a readable first frame");
    let msg = RendezvousMessage::parse_from_bytes(&bytes).expect("a RendezvousMessage");
    match msg.union {
        Some(rendezvous_message::Union::KeyExchange(ex)) => ex,
        other => panic!("expected a KeyExchange first, got {other:?}"),
    }
}

pub async fn secure(
    conn: &mut TcpClient,
    rs_pk: &sign::PublicKey,
    client: KxClient,
) -> Result<u32, String> {
    let ex = read_offer(conn).await;
    answer(conn, rs_pk, client, &ex).await
}

/// Lets a test read the offer at once and answer it after a (virtual) delay.
pub async fn answer(
    conn: &mut TcpClient,
    rs_pk: &sign::PublicKey,
    client: KxClient,
    offer: &KeyExchange,
) -> Result<u32, String> {
    let their_pk = verify_offer(offer, rs_pk, client)?;
    let picked = picked_by(client, offer.version);
    let reply = reply(their_pk, picked);
    conn.stream()
        .send(&reply.msg)
        .await
        .map_err(|e| e.to_string())?;
    install(conn, reply, their_pk, offer.version, picked)?;
    Ok(picked)
}

fn install(
    conn: &mut TcpClient,
    reply: Reply,
    their_pk: [u8; 32],
    advertised: u32,
    picked: u32,
) -> Result<(), String> {
    if picked == 0 {
        conn.stream().set_key(reply.key);
        return Ok(());
    }
    let transcript = KxTranscript {
        initiator_pk: &reply.our_pk,
        responder_pk: &their_pk,
        advertised,
        picked,
    };
    conn.stream()
        .set_key_split(reply.key, true, &transcript)
        .map_err(|e| e.to_string())
}

pub async fn secured_tcp(server: &super::TestServer) -> TcpClient {
    let mut conn = server.tcp().await;
    let picked = secure(&mut conn, &server.public_key, KxClient::Current)
        .await
        .expect("the server's offer passes the client's checks");
    assert_eq!(picked, 1);
    conn
}
