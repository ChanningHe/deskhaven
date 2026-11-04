use sea_orm::DbErr;

use super::{PasswordError, PolicyViolation};
use crate::console_audit::{GuardError, Refused};

/// Why a user operation failed. Variants are stable for API error mapping; messages never
/// contain passwords, and database details stay in the source chain.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum UserError {
    #[error("user not found")]
    NotFound,

    /// A unique value (`username` or `email`, compared case-insensitively) is taken.
    #[error("{field} is already taken")]
    Conflict { field: &'static str },

    /// Malformed input; `field` names the offending input field.
    #[error("invalid {field}: {reason}")]
    Invalid { field: &'static str, reason: String },

    #[error("password rejected: {reason}")]
    PolicyViolation { reason: PolicyViolation },

    /// The change would leave no active owner.
    #[error("the last active owner cannot be demoted, disabled or deleted")]
    LastOwner,

    /// Only `auth_source = local` accounts have a password.
    #[error("the account does not use a local password")]
    NotLocal,

    /// Self-service password change: the current password does not match.
    #[error("the current password is wrong")]
    WrongPassword,

    /// Re-authentication of a directory (LDAP) account: the directory could not be asked
    /// (nothing was verified; APIs: 503).
    #[error("the directory is unavailable")]
    DirectoryUnavailable,

    #[error("password hashing failed")]
    Password(#[source] PasswordError),

    /// A [`TxGuard`](crate::console_audit::TxGuard) declined the change (nothing was written).
    #[error(transparent)]
    Refused(#[from] Refused),

    #[error("the operating system random number generator failed")]
    Random,

    /// The operation's task panicked or the runtime is shutting down; whether it committed
    /// is unknown until the next read.
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

/// Why a local login failed.
///
/// Unknown user, wrong password and non-local account all yield
/// [`InvalidCredentials`](Self::InvalidCredentials) after the same amount of hashing work.
/// [`Disabled`](Self::Disabled) and [`Unverified`](Self::Unverified) are only returned once
/// the password has been verified, so they reveal nothing to someone without it.
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

    /// A directory (LDAP) account's directory could not be asked: unreachable, timed out,
    /// refused the service account, or misconfigured (logged). Nothing was verified, and
    /// there is no fallback to a local password (APIs: 503, not counted).
    #[error("the directory is unavailable")]
    DirectoryUnavailable,

    /// Like [`DirectoryUnavailable`](Self::DirectoryUnavailable), but the password had
    /// already been sent to the directory (a bind timed out or failed, or a later step did):
    /// the attempt counts (APIs: 503, counted, not refunded).
    #[error("the directory stopped answering after the password was sent")]
    DirectoryInterrupted,

    /// The directory accepted the password, but provisioning the account or synchronizing
    /// its role / groups failed (fail closed: no login).
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
