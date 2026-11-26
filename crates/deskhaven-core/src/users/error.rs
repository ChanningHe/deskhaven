use sea_orm::DbErr;

use super::{PasswordError, PolicyViolation};
use crate::console_audit::{GuardError, Refused};

/// Messages never contain passwords.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum UserError {
    #[error("user not found")]
    NotFound,

    #[error("{field} is already taken")]
    Conflict { field: &'static str },

    #[error("invalid {field}: {reason}")]
    Invalid { field: &'static str, reason: String },

    #[error("password rejected: {reason}")]
    PolicyViolation { reason: PolicyViolation },

    #[error("the last active owner cannot be demoted, disabled or deleted")]
    LastOwner,

    #[error("the account does not use a local password")]
    NotLocal,

    #[error("the current password is wrong")]
    WrongPassword,

    #[error("the directory is unavailable")]
    DirectoryUnavailable,

    #[error("password hashing failed")]
    Password(#[source] PasswordError),

    #[error(transparent)]
    Refused(#[from] Refused),

    #[error("the operating system random number generator failed")]
    Random,

    /// Whether it committed is unknown until the next read.
    #[error("user operation did not complete")]
    Aborted,

    #[error("user storage failed")]
    Database(#[source] DbErr),
}

impl From<DbErr> for UserError {
    fn from(err: DbErr) -> Self {
        Self::Database(err)
    }
}

impl From<PasswordError> for UserError {
    fn from(err: PasswordError) -> Self {
        Self::Password(err)
    }
}

impl From<GuardError> for UserError {
    fn from(err: GuardError) -> Self {
        match err {
            GuardError::Refused(refused) => Self::Refused(refused),
            GuardError::Database(db) => Self::Database(db),
        }
    }
}

/// `Disabled` and `Unverified` are only returned once the password has been verified, so they
/// reveal nothing to someone without it.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AuthError {
    #[error("invalid username or password")]
    InvalidCredentials,

    #[error("the account is disabled")]
    Disabled,

    #[error("the account is not verified")]
    Unverified,

    #[error("password hashing failed")]
    Password(#[source] PasswordError),

    /// There is no fallback to a local password.
    #[error("the directory is unavailable")]
    DirectoryUnavailable,

    /// The password was already sent, so the attempt counts.
    #[error("the directory stopped answering after the password was sent")]
    DirectoryInterrupted,

    /// Fail closed: no login.
    #[error("directory account provisioning failed")]
    Provisioning(#[source] Box<dyn std::error::Error + Send + Sync>),

    #[error("user storage failed")]
    Database(#[source] DbErr),
}

impl From<DbErr> for AuthError {
    fn from(err: DbErr) -> Self {
        Self::Database(err)
    }
}

impl From<PasswordError> for AuthError {
    fn from(err: PasswordError) -> Self {
        Self::Password(err)
    }
}
