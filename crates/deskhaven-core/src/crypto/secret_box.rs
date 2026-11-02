//! XChaCha20-Poly1305 sealing of secrets stored in the database.
//!
//! Sealed form: `v1:<key_id>:<nonce>:<ciphertext>` with unpadded standard
//! base64 fields and a random 24-byte nonce per seal. `v1:<key_id>` is bound
//! as associated data, so the header cannot be rewritten undetected.

use std::fmt;

use base64_022::Engine as _;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

use super::b64::SEALED_B64;
use super::{CryptoError, KeyId, MasterKey, random_bytes};

const VERSION: &str = "v1";
const SEPARATOR: char = ':';
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;

struct Slot {
    id: KeyId,
    cipher: XChaCha20Poly1305,
}

impl Slot {
    fn new(key: &MasterKey) -> Self {
        Self {
            id: key.key_id(),
            cipher: XChaCha20Poly1305::new(key.bytes().into()),
        }
    }
}

/// Seals with the primary key; opens with any held key (rotation). Cipher
/// state wipes its key copy on drop.
pub struct SecretBox {
    primary: Slot,
    previous: Vec<Slot>,
}

impl SecretBox {
    pub fn new(primary: &MasterKey) -> Self {
        Self {
            primary: Slot::new(primary),
            previous: Vec::new(),
        }
    }

    /// Adds a retired key that can still open existing ciphertexts.
    #[must_use]
    pub fn with_previous(self, key: &MasterKey) -> Self {
        let mut previous = self.previous;
        previous.push(Slot::new(key));
        Self {
            primary: self.primary,
            previous,
        }
    }

    pub fn seal(&self, plaintext: &[u8]) -> Result<String, CryptoError> {
        let nonce = random_bytes::<NONCE_LEN>()?;
        let header = header(self.primary.id);
        let payload = Payload {
            msg: plaintext,
            aad: header.as_bytes(),
        };
        let ciphertext = self
            .primary
            .cipher
            .encrypt(&XNonce::from(*nonce), payload)
            .map_err(|_| CryptoError::PlaintextTooLarge)?;
        Ok(format!(
            "{header}{SEPARATOR}{}{SEPARATOR}{}",
            SEALED_B64.encode(nonce.as_slice()),
            SEALED_B64.encode(ciphertext)
        ))
    }

    pub fn open(&self, sealed: &str) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
        let parsed = Sealed::parse(sealed)?;
        let slot = self.slot(parsed.key_id).ok_or(CryptoError::UnknownKeyId)?;
        let header = header(parsed.key_id);
        let payload = Payload {
            msg: &parsed.ciphertext,
            aad: header.as_bytes(),
        };
        slot.cipher
            .decrypt(&XNonce::from(parsed.nonce), payload)
            .map(Zeroizing::new)
            .map_err(|_| CryptoError::Authentication)
    }

    fn slot(&self, id: KeyId) -> Option<&Slot> {
        std::iter::once(&self.primary)
            .chain(&self.previous)
            .find(|slot| slot.id == id)
    }
}

impl fmt::Debug for SecretBox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let previous: Vec<KeyId> = self.previous.iter().map(|s| s.id).collect();
        f.debug_struct("SecretBox")
            .field("primary", &self.primary.id)
            .field("previous", &previous)
            .finish()
    }
}

fn header(id: KeyId) -> String {
    format!("{VERSION}{SEPARATOR}{id}")
}

struct Sealed {
    key_id: KeyId,
    nonce: [u8; NONCE_LEN],
    ciphertext: Vec<u8>,
}

impl Sealed {
    fn parse(text: &str) -> Result<Self, CryptoError> {
        let mut fields = text.split(SEPARATOR);
        let (Some(version), Some(key_id), Some(nonce), Some(ciphertext), None) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            return Err(CryptoError::Malformed);
        };
        if version != VERSION {
            return Err(CryptoError::Malformed);
        }
        let key_id = KeyId::parse(key_id).ok_or(CryptoError::Malformed)?;
        let nonce = decode(nonce)?
            .try_into()
            .map_err(|_| CryptoError::Malformed)?;
        let ciphertext = decode(ciphertext)?;
        if ciphertext.len() < TAG_LEN {
            return Err(CryptoError::Malformed);
        }
        Ok(Self {
            key_id,
            nonce,
            ciphertext,
        })
    }
}

fn decode(field: &str) -> Result<Vec<u8>, CryptoError> {
    SEALED_B64.decode(field).map_err(|_| CryptoError::Malformed)
}
