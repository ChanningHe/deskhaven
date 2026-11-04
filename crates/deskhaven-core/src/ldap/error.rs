use sea_orm::DbErr;

use crate::crypto::CryptoError;

/// Why a directory management operation failed. Variants are stable for API error mapping;
/// messages never contain the bind password.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DirectoryError {
    #[error("directory not found")]
    NotFound,

    /// `name` is taken, or (`enabled`) another directory is already enabled (P0: one).
    #[error("{field} conflicts with another directory")]
    Conflict { field: &'static str },

    #[error("invalid {field}: {reason}")]
    Invalid { field: &'static str, reason: String },

    /// The directory is defined in the configuration file and read-only here.
    #[error("the directory is managed by the configuration file")]
    ManagedByConfig,

    /// The bind password cannot be sealed or opened (wrong master key, moved ciphertext).
    #[error("bind password: {0}")]
    Crypto(#[source] CryptoError),

    #[error("directory storage failed")]
    Database(#[source] DbErr),
}

impl From<DbErr> for DirectoryError {
    fn from(err: DbErr) -> Self {
        Self::Database(err)
    }
}
