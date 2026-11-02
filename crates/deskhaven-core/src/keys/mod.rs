//! Key material on disk: the master key and the hbbs Ed25519 key pair.
//!
//! `read_*` functions never write; `load_or_generate_*` create missing keys
//! (atomically, never overwriting) inside a 0700 data directory.

mod fs;
mod master;
mod signal;
mod signal_pair;

use std::fmt;
use std::path::PathBuf;

use crate::crypto::CryptoError;

pub use master::{MASTER_KEY_FILE, load_or_generate_master_key, master_key_path, read_master_key};
pub use signal::{SIGNAL_KEY_FILE, SignalKeyPaths, load_or_generate_signal_key, read_signal_key};
pub use signal_pair::SignalKeyPair;

/// A loaded key plus non-fatal findings the caller should surface.
#[derive(Debug)]
pub struct Loaded<T> {
    pub value: T,
    pub warnings: Vec<KeyWarning>,
}

impl<T> Loaded<T> {
    fn new(value: T, warnings: Vec<KeyWarning>) -> Self {
        Self { value, warnings }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyWarning {
    /// A private key file is accessible to group or others.
    PermissiveMode { path: PathBuf, mode: u32 },
    /// The private key loaded but its `.pub` companion could not be written.
    PublicKeyNotWritten { path: PathBuf, reason: String },
}

impl fmt::Display for KeyWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PermissiveMode { path, mode } => write!(
                f,
                "{} has permissions {mode:04o}; private keys should be 0600 (chmod 600 {0})",
                path.display()
            ),
            Self::PublicKeyNotWritten { path, reason } => write!(
                f,
                "could not write {}: {reason}; continuing with the existing private key",
                path.display()
            ),
        }
    }
}

/// Errors name files and causes, never key material or file contents.
#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("security.master_key is invalid")]
    ConfiguredMasterKey(#[source] CryptoError),
    #[error("{}: invalid master key file", path.display())]
    MasterKeyFile {
        path: PathBuf,
        #[source]
        source: CryptoError,
    },
    #[error("{}: invalid hbbs private key: {reason}", path.display())]
    InvalidSignalKey { path: PathBuf, reason: &'static str },
    #[error("{} does not match the private key in {}", path.display(), secret.display())]
    PublicKeyMismatch { path: PathBuf, secret: PathBuf },
    #[error(
        "{} exists but its private key {} is missing; restore the private key or remove the stale public key",
        path.display(),
        secret.display()
    )]
    OrphanPublicKey { path: PathBuf, secret: PathBuf },
    #[error("failed to {op} {}", path.display())]
    Io {
        op: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to generate key material")]
    Generate(#[source] CryptoError),
}

/// `error: cause: cause...` for embedding an error chain in a warning.
fn render_chain(err: &dyn std::error::Error) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}
