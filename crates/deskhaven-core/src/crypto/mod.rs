//! Secret-at-rest encryption (06-security §5): the master key and `SecretBox`.

mod b64;
mod master_key;
mod secret_box;

use chacha20poly1305::aead::OsRng;
use chacha20poly1305::aead::rand_core::RngCore;
use zeroize::Zeroizing;

pub(crate) use b64::KEY_B64;
pub use master_key::{KeyId, MASTER_KEY_LEN, MasterKey};
pub use secret_box::SecretBox;

/// Errors never carry plaintext, ciphertext or key material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CryptoError {
    #[error("invalid master key: {0}")]
    InvalidMasterKey(&'static str),
    #[error("sealed secret is malformed")]
    Malformed,
    #[error("sealed secret uses an unknown key id (sealed under another master key?)")]
    UnknownKeyId,
    #[error("sealed secret failed authentication (tampered, or wrong master key)")]
    Authentication,
    #[error("plaintext is too large to seal")]
    PlaintextTooLarge,
    #[error("the operating system random number generator failed")]
    Random,
}

/// `N` bytes from the OS CSPRNG, wiped on drop.
pub(crate) fn random_bytes<const N: usize>() -> Result<Zeroizing<[u8; N]>, CryptoError> {
    let mut buf = Zeroizing::new([0u8; N]);
    OsRng
        .try_fill_bytes(buf.as_mut_slice())
        .map_err(|_| CryptoError::Random)?;
    Ok(buf)
}
