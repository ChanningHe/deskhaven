//! Key material on disk: the master key and the hbbs Ed25519 key pair.
//!
//! - `read_*` functions never write; `load_or_generate_*` create missing
//!   keys inside a 0700 directory and then read back whatever is on disk, so
//!   concurrent first starts converge on one key.
//! - Key files are created atomically and never overwritten: a temporary
//!   file is written, synced, then linked into place with
//!   `persist_noclobber`. That needs `renameat2(RENAME_NOREPLACE)` or hard
//!   links; some network or FUSE mounts (e.g. certain CIFS setups) support
//!   neither, and key generation fails there with an I/O error.
//! - Symlinked key files are refused (checked, then opened with
//!   `O_NOFOLLOW`); symlinks in parent directories are allowed. Key files
//!   larger than [`MAX_KEY_FILE_LEN`] are refused before reading.
//! - Group/other access to a private key, a foreign owner, or a
//!   group/other-writable key directory are reported as warnings; nothing
//!   is chmod-ed.

mod fs;
mod master;
mod master_guard;
mod read;
mod signal;
mod signal_pair;

use std::fmt;
use std::path::PathBuf;

use crate::crypto::CryptoError;

pub use master::{MASTER_KEY_FILE, load_or_generate_master_key, master_key_path, read_master_key};
pub use master_guard::{MasterKeyError, load_or_generate_master_key_for};
pub use read::MAX_KEY_FILE_LEN;
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
    /// A private key file is owned by another user than the process.
    ForeignOwner {
        path: PathBuf,
        owner: u32,
        euid: u32,
    },
    /// The directory holding a private key is writable by group or others.
    WritableDirectory { path: PathBuf, mode: u32 },
    /// The private key loaded but its `.pub` companion could not be written.
    PublicKeyNotWritten { path: PathBuf, reason: String },
    /// `security.master_key` is set and `master.key` holds another key.
    ConflictingMasterKeyFile { path: PathBuf },
}

impl fmt::Display for KeyWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PermissiveMode { path, mode } => write!(
                f,
                "{} has permissions {mode:04o}; private keys should be 0600 (chmod 600 {0})",
                path.display()
            ),
            Self::ForeignOwner { path, owner, euid } => write!(
                f,
                "{} is owned by uid {owner}, not by the server's uid {euid}",
                path.display()
            ),
            Self::WritableDirectory { path, mode } => write!(
                f,
                "key directory {} is group/other writable (mode {mode:04o}); \
                 others could replace key files (chmod go-w {0})",
                path.display()
            ),
            Self::PublicKeyNotWritten { path, reason } => write!(
                f,
                "could not write {}: {reason}; continuing with the existing private key",
                path.display()
            ),
            Self::ConflictingMasterKeyFile { path } => write!(
                f,
                "security.master_key is set, but {} holds a different or unreadable key; \
                 the file is ignored and secrets sealed with it cannot be opened",
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
    #[error("{} is a symlink; key files must be regular files (copy the key there instead)", path.display())]
    Symlink { path: PathBuf },
    #[error("{} is not a regular file", path.display())]
    NotRegularFile { path: PathBuf },
    #[error("{} is larger than {limit} bytes; not a key file", path.display())]
    TooLarge { path: PathBuf, limit: usize },
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

impl KeyError {
    fn io(op: &'static str, path: &std::path::Path, source: std::io::Error) -> Self {
        Self::Io {
            op,
            path: path.to_path_buf(),
            source,
        }
    }

    /// A key file disappeared between being written and read back.
    fn vanished(path: &std::path::Path) -> Self {
        Self::io("read back", path, std::io::ErrorKind::NotFound.into())
    }
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
