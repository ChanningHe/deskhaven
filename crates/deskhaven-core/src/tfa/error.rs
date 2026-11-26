//! None carries a seed, code or challenge secret.

use sea_orm::DbErr;

use crate::console_audit::{GuardError, Refused};
use crate::crypto::CryptoError;

#[derive(Debug, thiserror::Error)]
pub enum TfaError {
    #[error("user not found")]
    NotFound,
    #[error("two-factor authentication is already enabled")]
    AlreadyEnabled,
    #[error("two-factor authentication is not enabled")]
    NotEnabled,
    #[error("no two-factor enrolment is pending")]
    NoPendingEnrolment,
    #[error("invalid code")]
    InvalidCode,
    #[error("stored two-factor state is unreadable")]
    Corrupt,
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    #[error(transparent)]
    Refused(#[from] Refused),
    #[error(transparent)]
    Db(#[from] DbErr),
}

impl From<GuardError> for TfaError {
    fn from(err: GuardError) -> Self {
        match err {
            GuardError::Refused(refused) => Self::Refused(refused),
            GuardError::Database(db) => Self::Db(db),
        }
    }
}

/// `Invalid` and `WrongCode` are what the client may learn.
#[derive(Debug, thiserror::Error)]
pub enum ChallengeError {
    #[error("invalid or expired login challenge")]
    Invalid,
    /// The challenge stays usable until its attempts run out.
    #[error("wrong second-factor code")]
    WrongCode,
    #[error(transparent)]
    Internal(#[from] TfaError),
}

impl From<DbErr> for ChallengeError {
    fn from(err: DbErr) -> Self {
        Self::Internal(err.into())
    }
}
