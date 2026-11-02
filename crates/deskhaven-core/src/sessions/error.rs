use sea_orm::DbErr;

use crate::crypto::CryptoError;
use crate::users::UserError;

/// Why a session operation failed. Messages never contain tokens or token hashes.
///
/// For API mapping: [`is_unauthenticated`](Self::is_unauthenticated) errors are a 401 with
/// one generic message (which of them it was is for logs and tests only); everything else
/// is a server-side failure and must **not** become a 401 (a 401 logs the client out).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SessionError {
    /// Malformed token, or no session has it.
    #[error("invalid session token")]
    InvalidToken,

    #[error("session revoked")]
    Revoked,

    /// Past its absolute expiry, or idle for longer than `auth.session_idle_ttl`.
    #[error("session expired")]
    Expired,

    /// The owner is disabled, unverified or deleted (checked at read time).
    #[error("session owner is not active")]
    UserInactive,

    /// Creation: the credential the login verified changed (or the user stopped being an
    /// active local account) before the session could be stored. The login must restart.
    #[error("credentials changed during login")]
    StaleCredential,

    /// Creation: a client session needs a valid device id and uuid.
    #[error("invalid {field}: {reason}")]
    InvalidDevice {
        field: &'static str,
        reason: &'static str,
    },

    #[error("cannot generate a session token")]
    Random(#[source] CryptoError),

    /// The operation's task panicked or the runtime is shutting down; whether it committed
    /// is unknown until the next read.
    #[error("session operation did not complete")]
    Aborted,

    #[error("session storage failed")]
    Database(#[source] DbErr),
}

impl SessionError {
    /// Whether the presented token simply does not authenticate (HTTP 401).
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
