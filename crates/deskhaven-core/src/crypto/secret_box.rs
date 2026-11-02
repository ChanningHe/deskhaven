//! XChaCha20-Poly1305 sealing of secrets stored in the database.
//!
//! Sealed form: `v1:<key_id>:<nonce>:<ciphertext>` with unpadded standard
//! base64 fields and a random 24-byte nonce per seal.
//!
//! Associated data is `"v1:<key_id>:" || context`. The context names where
//! the ciphertext lives, by convention `"<table>.<column>:<row id>"` (e.g.
//! `users.totp_secret_enc:42`, 06-security §5), so an attacker with database
//! write access but no master key cannot move a ciphertext to another row or
//! column. The context is not stored; callers recompute it on open.

use std::fmt;

use base64_022::Engine as _;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

use super::b64::SEALED_B64;
use super::{CryptoError, KeyId, MasterKey, fill_random};

/// Largest plaintext `seal` accepts; sealed output stays below [`MAX_SEALED_LEN`].
pub const MAX_PLAINTEXT_LEN: usize = 512 * 1024;
/// Longer strings are refused by `open` before any decoding.
pub const MAX_SEALED_LEN: usize = 1024 * 1024;

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

    /// Adds a retired key that can still open existing ciphertexts. Each key
    /// id may be held once.
    pub fn with_previous(self, key: &MasterKey) -> Result<Self, CryptoError> {
        if self.slot(key.key_id()).is_some() {
            return Err(CryptoError::DuplicateKeyId);
        }
        let mut previous = self.previous;
        previous.push(Slot::new(key));
        Ok(Self {
            primary: self.primary,
            previous,
        })
    }

    pub fn seal(&self, context: &str, plaintext: &[u8]) -> Result<String, CryptoError> {
        if plaintext.len() > MAX_PLAINTEXT_LEN {
            return Err(CryptoError::PlaintextTooLarge);
        }
        let mut nonce = [0u8; NONCE_LEN];
        fill_random(&mut nonce)?;
        let header = header(self.primary.id);
        let aad = aad(&header, context);
        let payload = Payload {
            msg: plaintext,
            aad: aad.as_bytes(),
        };
        let ciphertext = self
            .primary
            .cipher
            .encrypt(&XNonce::from(nonce), payload)
            .map_err(|_| CryptoError::PlaintextTooLarge)?;
        Ok(format!(
            "{header}{SEPARATOR}{}{SEPARATOR}{}",
            SEALED_B64.encode(nonce),
            SEALED_B64.encode(ciphertext)
        ))
    }

    pub fn open(&self, context: &str, sealed: &str) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
        let parsed = Sealed::parse(sealed)?;
        let slot = self.slot(parsed.key_id).ok_or(CryptoError::UnknownKeyId)?;
        let aad = aad(&header(parsed.key_id), context);
        let payload = Payload {
            msg: &parsed.ciphertext,
            aad: aad.as_bytes(),
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

/// The key id has a fixed width, so `header ':' context` is unambiguous.
fn aad(header: &str, context: &str) -> String {
    format!("{header}{SEPARATOR}{context}")
}

struct Sealed {
    key_id: KeyId,
    nonce: [u8; NONCE_LEN],
    ciphertext: Vec<u8>,
}

impl Sealed {
    fn parse(text: &str) -> Result<Self, CryptoError> {
        if text.len() > MAX_SEALED_LEN {
            return Err(CryptoError::Malformed);
        }
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
