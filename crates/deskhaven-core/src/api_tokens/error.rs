use sea_orm::DbErr;

use crate::console_audit::{GuardError, Refused};
use crate::crypto::CryptoError;

/// Messages never contain tokens or token hashes.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ApiTokenError {
    #[error("invalid API token")]
    InvalidToken,

    #[error("API token revoked")]
    Revoked,

    #[error("API token expired")]
    Expired,

    /// The token carried its creator's authority, so it has none left.
    #[error("API token creator is not an active administrator")]
    CreatorInactive,

    #[error("API token lacks the required scope")]
    MissingScope,

    #[error("invalid {field}: {reason}")]
    Invalid { field: &'static str, reason: String },

    #[error("creator not found")]
    CreatorNotFound,

    #[error(transparent)]
    Refused(#[from] Refused),

    #[error("cannot generate an API token")]
    Random(#[source] CryptoError),

    #[error("API token storage failed")]
    Database(#[from] DbErr),
}

impl From<GuardError> for ApiTokenError {
    fn from(err: GuardError) -> Self {
        match err {
            GuardError::Refused(refused) => Self::Refused(refused),
            GuardError::Database(db) => Self::Database(db),
        }
    }
}

impl ApiTokenError {
    pub fn is_unauthenticated(&self) -> bool {
        matches!(
            self,
            Self::InvalidToken
                | Self::Revoked
                | Self::Expired
                | Self::CreatorInactive
                | Self::MissingScope
        )
    }
}
