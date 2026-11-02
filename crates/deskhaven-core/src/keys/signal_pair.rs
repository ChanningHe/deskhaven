//! The hbbs Ed25519 key pair in upstream's representation.
//!
//! Upstream rustdesk-server (`src/common.rs` `gen_sk`) stores a sodiumoxide
//! `sign::SecretKey`: 64 bytes = 32-byte seed || 32-byte public key, as
//! padded standard base64, and the public key as base64 of its 32 bytes.
//! Ed25519 (RFC 8032) is identical in libsodium and ed25519-dalek, so the
//! same seed yields the same public key and the same signatures.

use std::fmt;

use base64_022::Engine as _;
use ed25519_dalek::{KEYPAIR_LENGTH, PUBLIC_KEY_LENGTH, SECRET_KEY_LENGTH, Signer, SigningKey};
use zeroize::Zeroizing;

use crate::crypto::{CryptoError, KEY_B64, random_bytes};

/// Upstream retries generation this many times (common.rs `gen_sk`).
const GENERATE_ATTEMPTS: usize = 300;

/// Wiped on drop (ed25519-dalek `zeroize`); `Debug` shows only the public key.
pub struct SignalKeyPair {
    signing: SigningKey,
}

impl SignalKeyPair {
    /// A fresh pair whose base64 public key contains neither '/' nor ':'
    /// (clients take the key from a renamed exe file name), as upstream.
    pub fn generate() -> Result<Self, CryptoError> {
        let mut pair = Self::random()?;
        for _ in 1..GENERATE_ATTEMPTS {
            if is_file_name_safe(&pair.public_key_base64()) {
                break;
            }
            pair = Self::random()?;
        }
        Ok(pair)
    }

    fn random() -> Result<Self, CryptoError> {
        let seed = random_bytes::<SECRET_KEY_LENGTH>()?;
        Ok(Self {
            signing: SigningKey::from_bytes(&seed),
        })
    }

    /// Parses the contents of an upstream `id_ed25519` file. The stored
    /// public half must match the one derived from the seed.
    pub(crate) fn from_secret_base64(text: &str) -> Result<Self, &'static str> {
        let decoded = KEY_B64
            .decode(text.trim())
            .map(Zeroizing::new)
            .map_err(|_| "not valid base64")?;
        let bytes: &[u8; KEYPAIR_LENGTH] = decoded
            .as_slice()
            .try_into()
            .map_err(|_| "expected 64 bytes (seed || public key)")?;
        SigningKey::from_keypair_bytes(bytes)
            .map(|signing| Self { signing })
            .map_err(|_| "public half does not match the seed")
    }

    /// Contents of an upstream `id_ed25519` file.
    pub fn secret_base64(&self) -> Zeroizing<String> {
        Zeroizing::new(KEY_B64.encode(self.keypair_bytes().as_slice()))
    }

    /// Seed || public key: the libsodium / sodiumoxide `sign::SecretKey` layout.
    pub fn keypair_bytes(&self) -> Zeroizing<[u8; KEYPAIR_LENGTH]> {
        Zeroizing::new(self.signing.to_keypair_bytes())
    }

    pub fn public_key(&self) -> [u8; PUBLIC_KEY_LENGTH] {
        self.signing.verifying_key().to_bytes()
    }

    /// The `Key` value clients are configured with; contents of `id_ed25519.pub`.
    pub fn public_key_base64(&self) -> String {
        KEY_B64.encode(self.public_key())
    }

    /// Detached Ed25519 signature (libsodium `crypto_sign_detached`).
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.signing.sign(message).to_bytes()
    }
}

fn is_file_name_safe(public_key: &str) -> bool {
    !public_key.contains('/') && !public_key.contains(':')
}

impl fmt::Debug for SignalKeyPair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignalKeyPair")
            .field("public_key", &self.public_key_base64())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 8032 §7.1 TEST 1 (the vector libsodium is tested against).
    const RFC8032_SEED: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
    const RFC8032_PUBLIC: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
    const RFC8032_SIGNATURE: &str = "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155\
                                     5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b";

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    fn rfc_pair() -> SignalKeyPair {
        let mut keypair = hex(RFC8032_SEED);
        keypair.extend(hex(RFC8032_PUBLIC));
        SignalKeyPair::from_secret_base64(&KEY_B64.encode(&keypair)).unwrap()
    }

    #[test]
    fn rfc8032_vector_derives_public_key_and_signature() {
        let pair = rfc_pair();
        assert_eq!(pair.public_key().to_vec(), hex(RFC8032_PUBLIC));
        assert_eq!(pair.sign(b"").to_vec(), hex(RFC8032_SIGNATURE));
        assert_eq!(&pair.keypair_bytes()[..32], hex(RFC8032_SEED).as_slice());
    }

    #[test]
    fn secret_base64_round_trips_and_is_padded() {
        let pair = rfc_pair();
        let text = pair.secret_base64();
        assert_eq!(text.len(), 88);
        assert!(text.ends_with("=="));
        let again = SignalKeyPair::from_secret_base64(&text).map(|p| p.public_key());
        assert_eq!(again, Ok(pair.public_key()));
    }

    #[test]
    fn generated_public_keys_are_file_name_safe() {
        for _ in 0..32 {
            let key = SignalKeyPair::generate().unwrap().public_key_base64();
            assert!(is_file_name_safe(&key) && key.len() == 44, "{key}");
        }
    }
}
