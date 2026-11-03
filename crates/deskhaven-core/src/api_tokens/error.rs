use sea_orm::DbErr;

use crate::crypto::CryptoError;

/// Why an API token operation failed. Messages never contain tokens or token hashes.
///
/// For API mapping: [`is_unauthenticated`](Self::is_unauthenticated) errors are a 401 with
/// one generic message (which one it was is for logs and tests only); everything else is a
/// server-side failure, except [`Invalid`](Self::Invalid) and
/// [`CreatorNotFound`](Self::CreatorNotFound) (bad creation input).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ApiTokenError {
    /// Malformed or unknown token.
    #[error("invalid API token")]
    InvalidToken,

    #[error("API token revoked")]
    Revoked,

    #[error("API token expired")]
    Expired,

    /// The creator is disabled, unverified or no longer an administrator: the token carried
    /// their authority, so it has none left. From `create`: such an account cannot create
    /// one (a request error there, not a 401).
    #[error("API token creator is not an active administrator")]
    CreatorInactive,

    /// The token lacks the scope the operation needs.
    #[error("API token lacks the required scope")]
    MissingScope,

    /// Creation: malformed input; `field` names the offending field.
    #[error("invalid {field}: {reason}")]
    Invalid { field: &'static str, reason: String },

    /// Creation: the creating user does not exist.
    #[error("creator not found")]
    CreatorNotFound,

    #[error("cannot generate an API token")]
    Random(#[source] CryptoError),

    #[error("API token storage failed")]
    Database(#[from] DbErr),
}

impl ApiTokenError {
    /// Whether the presented token simply does not authorize the request (HTTP 401).
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
