use sea_orm::DbErr;

use crate::console_audit::{GuardError, Refused};
use crate::crypto::CryptoError;
use crate::users::UserError;

/// Messages never contain tokens or token hashes. Anything not `is_unauthenticated` must not become
/// a 401: a 401 logs the client out.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SessionError {
    #[error("invalid session token")]
    InvalidToken,

    #[error("session revoked")]
    Revoked,

    #[error("session expired")]
    Expired,

    #[error("session owner is not active")]
    UserInactive,

    /// The login must restart.
    #[error("credentials changed during login")]
    StaleCredential,

    #[error("invalid {field}: {reason}")]
    InvalidDevice {
        field: &'static str,
        reason: &'static str,
    },

    #[error("cannot generate a session token")]
    Random(#[source] CryptoError),

    #[error("invalid {field}: {reason}")]
    Invalid { field: &'static str, reason: String },

    #[error("user not found")]
    UserNotFound,

    #[error(transparent)]
    Refused(#[from] Refused),

    /// Whether it committed is unknown until the next read.
    #[error("session operation did not complete")]
    Aborted,

    #[error("session storage failed")]
    Database(#[source] DbErr),
}

impl SessionError {
    pub fn is_unauthenticated(&self) -> bool {
        matches!(
            self,
            Self::InvalidToken | Self::Revoked | Self::Expired | Self::UserInactive
        )
    }
}

impl From<DbErr> for SessionError {
    fn from(err: DbErr) -> Self {
        Self::Database(err)
    }
}

impl From<GuardError> for SessionError {
    fn from(err: GuardError) -> Self {
        match err {
            GuardError::Refused(refused) => Self::Refused(refused),
            GuardError::Database(db) => Self::Database(db),
        }
    }
}

impl From<UserError> for SessionError {
    fn from(err: UserError) -> Self {
        match err {
            UserError::Database(db) => Self::Database(db),
            UserError::Aborted => Self::Aborted,
            // `credential_stamp_matches` fails with storage errors only.
            other => Self::Database(DbErr::Custom(other.to_string())),
        }
    }
}
