//! Errors of the 2FA service. None carries a seed, code or challenge secret.

use sea_orm::DbErr;

use crate::crypto::CryptoError;

/// Enrolment and management errors.
#[derive(Debug, thiserror::Error)]
pub enum TfaError {
    #[error("user not found")]
    NotFound,
    /// Enrolment would replace an active seed: disable 2FA first.
    #[error("two-factor authentication is already enabled")]
    AlreadyEnabled,
    #[error("two-factor authentication is not enabled")]
    NotEnabled,
    /// `confirm_enrolment` without a preceding `begin_enrolment`.
    #[error("no two-factor enrolment is pending")]
    NoPendingEnrolment,
    /// The confirmation code does not match the pending seed.
    #[error("invalid code")]
    InvalidCode,
    /// A stored seed or challenge does not decode (corrupt row, or another master key).
    #[error("stored two-factor state is unreadable")]
    Corrupt,
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    #[error(transparent)]
    Db(#[from] DbErr),
}

/// Why the second login step failed. `Invalid` and `WrongCode` are what the client may
/// learn; `Internal` is a server failure (logged by the caller).
#[derive(Debug, thiserror::Error)]
pub enum ChallengeError {
    /// Unknown, expired, already used or exhausted challenge, or one issued to another
    /// username or device: the client must log in again.
    #[error("invalid or expired login challenge")]
    Invalid,
    /// Wrong, replayed (same or older time step) or already used code; the challenge stays
    /// usable until its attempts run out.
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
