use sea_orm::DbErr;

use super::{PasswordError, PolicyViolation};

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

    #[error("password hashing failed")]
    Password(#[source] PasswordError),

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
