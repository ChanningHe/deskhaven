//! deskhaven (M1-07): the server half of the KeyExchange on rendezvous TCP connections
//! (07 §4). Not vendored: upstream hbbs has no key exchange. Written from the protocol as the
//! client runs it (RustDesk 23e5bb4 src/common.rs `key_exchange`, hbb_common fae47e0 tcp.rs
//! `Encrypt`); see CHANGES.md for the reference implementations consulted.
//!
//! 1. Right after accept the server sends `KeyExchange{keys: [sign(S_pk)], version: 1,
//!    signed_params: sign("rdkx-params" || KxParams{pk: S_pk, version: 1})}`, signed with the
//!    hbbs ed25519 key, where `S_pk` is a fresh X25519 key of this connection with bit 255
//!    set. X25519 ignores that bit, so clients from before v1 use the key as is; current
//!    clients take it as "the parameters are signed" and refuse the offer without them, and
//!    since the bit is inside the signature it cannot be stripped.
//! 2. A client that runs the exchange answers `KeyExchange{keys: [C_pk, box(sym, nonce 0,
//!    S_pk, C_sk)], version: picked}` with `picked <= 1` (absent = 0 for clients before v1).
//! 3. From then on every non-empty frame is a secretbox with a per-direction little-endian
//!    counter nonce (hbb_common `Encrypt`): one key both ways for v0, for v1 one key per
//!    direction derived from `sym` and the transcript (`Encrypt::new_split`; the server is the
//!    responder). Empty frames (heartbeats) stay plain both ways.

use hbb_common::{
    bytes::{Bytes, BytesMut},
    protobuf::Message as _,
    rendezvous_proto::{KeyExchange, KxParams, RendezvousMessage, rendezvous_message},
    tcp::{Encrypt, KX_PARAMS_DOMAIN, KX_VERSION_LATEST, KxTranscript},
};
use sodiumoxide::crypto::{box_, sign};

/// The version the server advertises: the newest it speaks.
pub(crate) const KX_VERSION: u32 = 1;
// hbb_common must implement every version advertised here (`Encrypt::new_split` is v1).
const _: () = assert!(KX_VERSION <= KX_VERSION_LATEST);
/// Bit 255 of the advertised X25519 key: "signed parameters required" (client common.rs).
const SIGNED_PARAMS_FLAG: u8 = 0x80;
/// Keys in a client's reply: its public key and the sealed symmetric key.
const REPLY_KEYS: usize = 2;

/// Why a KeyExchange reply was refused; the connection is closed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum KxError {
    #[error("the KeyExchange offer could not be encoded")]
    Encode,
    #[error("KeyExchange reply with {0} keys")]
    Keys(usize),
    #[error("KeyExchange reply picked version {0}, above the advertised {KX_VERSION}")]
    Version(u32),
    #[error("KeyExchange reply picked version 0 while require_kx_v1 is set")]
    V0Refused,
    #[error("KeyExchange reply does not open with this connection's key")]
    Open,
    #[error("frame does not decrypt")]
    Decrypt,
}

/// The server's side of one connection's exchange, from the offer to the client's reply.
pub(crate) struct Offer {
    sk: box_::SecretKey,
    /// The public key as advertised and signed (bit 255 set): the transcript uses it so.
    pk: [u8; box_::PUBLICKEYBYTES],
}

impl Offer {
    /// A fresh ephemeral key pair and the KeyExchange advertising it, signed with `server_sk`.
    pub(crate) fn new(server_sk: &sign::SecretKey) -> Result<(Self, RendezvousMessage), KxError> {
        let (pk, sk) = box_::gen_keypair();
        let mut pk = pk.0;
        pk[31] |= SIGNED_PARAMS_FLAG;
        let params = KxParams {
            pk: Bytes::copy_from_slice(&pk),
            version: KX_VERSION,
            ..Default::default()
        }
        .write_to_bytes()
        .map_err(|_| KxError::Encode)?;
        let mut signed_params = KX_PARAMS_DOMAIN.to_vec();
        signed_params.extend_from_slice(&params);
        let mut msg = RendezvousMessage::new();
        msg.set_key_exchange(KeyExchange {
            keys: vec![sign::sign(&pk, server_sk).into()],
            version: KX_VERSION,
            signed_params: sign::sign(&signed_params, server_sk).into(),
            ..Default::default()
        });
        Ok((Self { sk, pk }, msg))
    }

    /// The cipher of the connection if `frame` is the client's KeyExchange reply, `None` if it
    /// is any other message (a client that skips the exchange: the connection stays plain).
    pub(crate) fn accept(
        &self,
        frame: &[u8],
        require_v1: bool,
    ) -> Result<Option<Encrypt>, KxError> {
        let Ok(msg) = RendezvousMessage::parse_from_bytes(frame) else {
            return Ok(None);
        };
        let Some(rendezvous_message::Union::KeyExchange(ex)) = msg.union else {
            return Ok(None);
        };
        if ex.keys.len() != REPLY_KEYS {
            return Err(KxError::Keys(ex.keys.len()));
        }
        let picked = ex.version;
        if picked > KX_VERSION {
            return Err(KxError::Version(picked));
        }
        if picked == 0 && require_v1 {
            return Err(KxError::V0Refused);
        }
        let (client_pk, sealed) = (&ex.keys[0], &ex.keys[1]);
        let key = Encrypt::decode(sealed, client_pk, &self.sk).map_err(|_| KxError::Open)?;
        if picked == 0 {
            return Ok(Some(Encrypt::new(key)));
        }
        let transcript = KxTranscript {
            initiator_pk: client_pk,
            responder_pk: &self.pk,
            advertised: KX_VERSION,
            picked,
        };
        Encrypt::new_split(key, false, &transcript)
            .map(Some)
            .map_err(|_| KxError::Open)
    }
}

/// The receiving side of one TCP connection's exchange.
pub(crate) enum KxState {
    /// The offer is out; the first frame tells whether the client runs the exchange.
    Offered(Offer),
    /// Every frame is opened with this (receiving) cipher.
    Secured(Encrypt),
    /// No exchange (websocket, or a client that skipped the offer): frames are plain.
    Plain,
}

/// A received frame after [`KxState::on_frame`].
pub(crate) enum Frame {
    /// A message for the rendezvous handler (opened if the connection is secured).
    Message(Bytes),
    /// The client's KeyExchange reply: the connection is secured from now on, and this is the
    /// sending cipher for its sink.
    Secured(Encrypt),
}

impl KxState {
    /// Offers the exchange on a new connection: the state and the KeyExchange to send first.
    pub(crate) fn offer(server_sk: &sign::SecretKey) -> Result<(Self, RendezvousMessage), KxError> {
        let (offer, msg) = Offer::new(server_sk)?;
        Ok((Self::Offered(offer), msg))
    }

    /// Takes in the next frame. Only the first frame after the offer may be the reply; an
    /// error closes the connection.
    pub(crate) fn on_frame(&mut self, frame: Bytes, require_v1: bool) -> Result<Frame, KxError> {
        match self {
            Self::Offered(offer) => match offer.accept(&frame, require_v1)? {
                Some(cipher) => {
                    // Each copy only ever uses one direction's key and counter.
                    *self = Self::Secured(cipher.clone());
                    Ok(Frame::Secured(cipher))
                }
                None => {
                    *self = Self::Plain;
                    Ok(Frame::Message(frame))
                }
            },
            Self::Secured(cipher) => open(Some(cipher), frame).map(Frame::Message),
            Self::Plain => Ok(Frame::Message(frame)),
        }
    }
}

/// `frame` as it goes out on a connection with `cipher`: sealed, except empty frames
/// (heartbeats), which clients skip when decrypting (`Encrypt::dec`: frames of at most one
/// byte) and expect plain. Sealing advances the sending counter, so frames must go out in the
/// order they were sealed (the connection's sink is the only writer).
pub(crate) fn seal(cipher: Option<&mut Encrypt>, frame: Bytes) -> Bytes {
    match cipher {
        Some(cipher) if !frame.is_empty() => Bytes::from(cipher.enc(&frame)),
        _ => frame,
    }
}

/// `frame` as received on a connection with `cipher`: opened, except frames of at most one
/// byte, which clients send plain (heartbeat answers) and never seal (a sealed frame carries a
/// 16-byte tag).
pub(crate) fn open(cipher: Option<&mut Encrypt>, frame: Bytes) -> Result<Bytes, KxError> {
    match cipher {
        Some(cipher) if frame.len() > 1 => {
            let mut buf = BytesMut::from(&frame[..]);
            cipher.dec(&mut buf).map_err(|_| KxError::Decrypt)?;
            Ok(buf.freeze())
        }
        _ => Ok(frame),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sodiumoxide::crypto::secretbox;

    fn offer() -> (Offer, KeyExchange, sign::PublicKey) {
        let (server_pk, server_sk) = sign::gen_keypair();
        let (offer, msg) = Offer::new(&server_sk).unwrap();
        (offer, msg.key_exchange().clone(), server_pk)
    }

    /// The client's reply to `ex` picking `picked`, and its initiator cipher.
    fn reply(ex: &KeyExchange, server_pk: &sign::PublicKey, picked: u32) -> (Vec<u8>, Encrypt) {
        let their_pk: [u8; 32] = sign::verify(&ex.keys[0], server_pk)
            .unwrap()
            .try_into()
            .unwrap();
        let (our_pk, our_sk) = box_::gen_keypair();
        let key = secretbox::gen_key();
        let nonce = box_::Nonce([0u8; box_::NONCEBYTES]);
        let sealed = box_::seal(&key.0, &nonce, &box_::PublicKey(their_pk), &our_sk);
        let mut msg = RendezvousMessage::new();
        msg.set_key_exchange(KeyExchange {
            keys: vec![our_pk.0.to_vec().into(), sealed.into()],
            version: picked,
            ..Default::default()
        });
        let cipher = if picked == 0 {
            Encrypt::new(key)
        } else {
            let t = KxTranscript {
                initiator_pk: &our_pk.0,
                responder_pk: &their_pk,
                advertised: ex.version,
                picked,
            };
            Encrypt::new_split(key, true, &t).unwrap()
        };
        (msg.write_to_bytes().unwrap(), cipher)
    }

    #[test]
    fn offer_signs_a_flagged_key_and_its_parameters() {
        let (offer, ex, server_pk) = offer();
        let pk = sign::verify(&ex.keys[0], &server_pk).unwrap();
        assert_eq!(pk, offer.pk);
        assert_eq!(pk[31] & SIGNED_PARAMS_FLAG, SIGNED_PARAMS_FLAG);
        let signed = sign::verify(&ex.signed_params, &server_pk).unwrap();
        let params =
            KxParams::parse_from_bytes(signed.strip_prefix(KX_PARAMS_DOMAIN).unwrap()).unwrap();
        assert_eq!((&params.pk[..], params.version), (&pk[..], KX_VERSION));
        assert_eq!(ex.version, KX_VERSION);
    }

    #[test]
    fn both_versions_give_matching_ciphers_both_ways() {
        for picked in [0, 1] {
            let (offer, ex, server_pk) = offer();
            let (frame, mut client) = reply(&ex, &server_pk, picked);
            let mut server = offer.accept(&frame, false).unwrap().unwrap();
            let up = seal(Some(&mut client), Bytes::from_static(b"up"));
            assert_eq!(open(Some(&mut server), up).unwrap(), &b"up"[..]);
            let down = seal(Some(&mut server), Bytes::from_static(b"down"));
            assert_eq!(open(Some(&mut client), down).unwrap(), &b"down"[..]);
        }
    }

    #[test]
    fn other_messages_are_not_a_reply() {
        let (offer, _, _) = offer();
        assert!(offer.accept(b"", false).unwrap().is_none());
        assert!(offer.accept(b"\xff\xff", false).unwrap().is_none());
        let mut msg = RendezvousMessage::new();
        msg.set_test_nat_request(Default::default());
        let frame = msg.write_to_bytes().unwrap();
        assert!(offer.accept(&frame, false).unwrap().is_none());
    }

    #[test]
    fn bad_replies_are_refused() {
        let (offer, ex, server_pk) = offer();
        let (frame, _) = reply(&ex, &server_pk, 2);
        assert!(matches!(
            offer.accept(&frame, false),
            Err(KxError::Version(2))
        ));
        let (frame, _) = reply(&ex, &server_pk, 0);
        assert!(matches!(
            offer.accept(&frame, true),
            Err(KxError::V0Refused)
        ));
        // Sealed to another connection's key.
        let (_other, other_ex, other_server_pk) = self::offer();
        let (frame, _) = reply(&other_ex, &other_server_pk, 1);
        assert!(matches!(offer.accept(&frame, false), Err(KxError::Open)));
        let mut msg = RendezvousMessage::parse_from_bytes(&frame).unwrap();
        msg.mut_key_exchange().keys.truncate(1);
        let frame = msg.write_to_bytes().unwrap();
        assert!(matches!(offer.accept(&frame, false), Err(KxError::Keys(1))));
    }

    #[test]
    fn empty_frames_stay_plain_and_garbage_does_not_open() {
        let key = secretbox::gen_key();
        let (mut a, mut b) = (Encrypt::new(key.clone()), Encrypt::new(key));
        assert!(seal(Some(&mut a), Bytes::new()).is_empty());
        assert!(open(Some(&mut b), Bytes::new()).unwrap().is_empty());
        assert_eq!(
            open(Some(&mut b), Bytes::from_static(b"x")).unwrap(),
            &b"x"[..]
        );
        assert!(matches!(
            open(Some(&mut b), Bytes::from_static(b"not sealed at all")),
            Err(KxError::Decrypt)
        ));
        assert_eq!(seal(None, Bytes::from_static(b"plain")), &b"plain"[..]);
    }
}
