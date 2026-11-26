use sea_orm::DbErr;

use crate::console_audit::{GuardError, Refused};
use crate::crypto::CryptoError;

/// Messages never contain the bind password.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DirectoryError {
    #[error("directory not found")]
    NotFound,

    #[error("{field} conflicts with another directory")]
    Conflict { field: &'static str },

    #[error("invalid {field}: {reason}")]
    Invalid { field: &'static str, reason: String },

    #[error("the directory is managed by the configuration file")]
    ManagedByConfig,

    #[error("bind password: {0}")]
    Crypto(#[source] CryptoError),

    #[error(transparent)]
    Refused(#[from] Refused),

    #[error("directory storage failed")]
    Database(#[source] DbErr),
}

impl From<DbErr> for DirectoryError {
    fn from(err: DbErr) -> Self {
        Self::Database(err)
    }
}

impl From<GuardError> for DirectoryError {
    fn from(err: GuardError) -> Self {
        match err {
            GuardError::Refused(refused) => Self::Refused(refused),
            GuardError::Database(db) => Self::Database(db),
        }
    }
}
