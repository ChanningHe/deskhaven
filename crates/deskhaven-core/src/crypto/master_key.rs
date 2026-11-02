//! The 32-byte master key and its public fingerprint.

use std::fmt;

use base64_022::Engine as _;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::{CryptoError, KEY_B64, random_bytes};

pub const MASTER_KEY_LEN: usize = 32;
const KEY_ID_LEN: usize = 4;
/// Domain separation: the fingerprint is never a plain hash of the key.
const KEY_ID_DOMAIN: &[u8] = b"deskhaven/master-key-id/v1\0";

/// Short stable fingerprint of a master key (8 lowercase hex digits). Names
/// the key inside sealed strings so several keys can coexist during rotation.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyId([u8; KEY_ID_LEN]);

impl KeyId {
    fn of(key: &[u8; MASTER_KEY_LEN]) -> Self {
        let digest = Sha256::new()
            .chain_update(KEY_ID_DOMAIN)
            .chain_update(key)
            .finalize();
        let mut id = [0u8; KEY_ID_LEN];
        id.copy_from_slice(&digest[..KEY_ID_LEN]);
        Self(id)
    }

    /// Accepts exactly the canonical rendering (lowercase hex).
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let (pairs, rest) = text.as_bytes().as_chunks::<2>();
        if pairs.len() != KEY_ID_LEN || !rest.is_empty() {
            return None;
        }
        let mut id = [0u8; KEY_ID_LEN];
        for (byte, [high, low]) in id.iter_mut().zip(pairs) {
            *byte = hex_digit(*high)? << 4 | hex_digit(*low)?;
        }
        Some(Self(id))
    }
}

fn hex_digit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

impl fmt::Display for KeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

impl fmt::Debug for KeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "KeyId({self})")
    }
}

/// Encrypts secrets at rest. Wiped on drop; `Debug` shows only the key id.
pub struct MasterKey {
    bytes: Zeroizing<[u8; MASTER_KEY_LEN]>,
    id: KeyId,
}

impl MasterKey {
    pub fn from_bytes(bytes: [u8; MASTER_KEY_LEN]) -> Self {
        let bytes = Zeroizing::new(bytes);
        let id = KeyId::of(&bytes);
        Self { bytes, id }
    }

    pub fn generate() -> Result<Self, CryptoError> {
        random_bytes::<MASTER_KEY_LEN>().map(|bytes| Self::from_bytes(*bytes))
    }

    /// Parses standard base64 (surrounding whitespace ignored). Errors never
    /// echo the input.
    pub fn from_base64(text: &str) -> Result<Self, CryptoError> {
        let decoded = KEY_B64
            .decode(text.trim())
            .map(Zeroizing::new)
            .map_err(|_| CryptoError::InvalidMasterKey("not valid base64"))?;
        let bytes: &[u8; MASTER_KEY_LEN] = decoded
            .as_slice()
            .try_into()
            .map_err(|_| CryptoError::InvalidMasterKey("must decode to exactly 32 bytes"))?;
        Ok(Self::from_bytes(*bytes))
    }

    /// Padded standard base64, the format of `security.master_key` and `master.key`.
    pub fn to_base64(&self) -> Zeroizing<String> {
        Zeroizing::new(KEY_B64.encode(self.bytes.as_slice()))
    }

    pub fn key_id(&self) -> KeyId {
        self.id
    }

    pub(crate) fn bytes(&self) -> &[u8; MASTER_KEY_LEN] {
        &self.bytes
    }
}

impl fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MasterKey")
            .field("key_id", &self.id.to_string())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_id_renders_and_parses_canonically() {
        let id = MasterKey::from_bytes([1; MASTER_KEY_LEN]).key_id();
        let text = id.to_string();
        assert_eq!(text.len(), 8);
        assert_eq!(KeyId::parse(&text), Some(id));
    }

    #[test]
    fn key_id_parse_rejects_non_hex_and_wrong_length() {
        for bad in ["", "abc", "abcdefg", "abcdefgh", "ABCDEF01", "abcdef012"] {
            assert_eq!(KeyId::parse(bad), None, "{bad}");
        }
        assert_eq!(
            KeyId::parse("00ff10a9").map(|k| k.to_string()).as_deref(),
            Some("00ff10a9")
        );
    }
}
