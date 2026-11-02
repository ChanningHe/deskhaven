//! Secret-at-rest encryption (06-security §5): the master key and `SecretBox`.

pub(crate) mod b64;
mod mac;
mod master_key;
mod secret_box;

use chacha20poly1305::aead::OsRng;
use chacha20poly1305::aead::rand_core::RngCore;

pub(crate) use b64::KEY_B64;
pub use master_key::{KeyId, MASTER_KEY_LEN, MasterKey};
pub use secret_box::{MAX_PLAINTEXT_LEN, MAX_SEALED_LEN, SecretBox};

/// Errors never carry plaintext, ciphertext or key material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CryptoError {
    #[error("invalid master key: {0}")]
    InvalidMasterKey(&'static str),
    #[error("sealed secret is malformed")]
    Malformed,
    #[error("sealed secret uses an unknown key id (sealed under another master key?)")]
    UnknownKeyId,
    #[error("sealed secret failed authentication (tampered, wrong context, or wrong master key)")]
    Authentication,
    #[error("plaintext is too large to seal")]
    PlaintextTooLarge,
    #[error("the same master key was added to the SecretBox twice")]
    DuplicateKeyId,
    #[error("the operating system random number generator failed")]
    Random,
}

/// Fills `buf` in place from the OS CSPRNG (no intermediate copies).
pub(crate) fn fill_random(buf: &mut [u8]) -> Result<(), CryptoError> {
    OsRng.try_fill_bytes(buf).map_err(|_| CryptoError::Random)
}
