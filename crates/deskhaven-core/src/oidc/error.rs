//! Messages never contain secrets (client secret, state, code, verifier, tokens); upstream
//! details stay in the source chain and the log.

use sea_orm::DbErr;

use crate::console_audit::{GuardError, Refused};
use crate::crypto::CryptoError;
use crate::login_events::LoginResult;
use crate::users::UserError;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ProviderError {
    #[error("OIDC provider not found")]
    NotFound,
    #[error("an OIDC provider named like this already exists")]
    Conflict,
    #[error("invalid {field}: {reason}")]
    Invalid { field: &'static str, reason: String },
    #[error("the OIDC provider is managed by the configuration file")]
    ManagedByConfig,
    #[error("the OIDC provider still has linked accounts")]
    InUse,
    #[error("sealing the client secret failed")]
    Crypto(#[source] CryptoError),
    #[error(transparent)]
    Refused(#[from] Refused),
    #[error("OIDC provider storage failed")]
    Database(#[source] DbErr),
}

impl From<DbErr> for ProviderError {
    fn from(err: DbErr) -> Self {
        Self::Database(err)
    }
}

impl From<GuardError> for ProviderError {
    fn from(err: GuardError) -> Self {
        match err {
            GuardError::Refused(refused) => Self::Refused(refused),
            GuardError::Database(db) => Self::Database(db),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OidcError {
    #[error("OIDC login is not configured")]
    NotConfigured,
    #[error("unknown OIDC provider")]
    UnknownProvider,
    #[error("invalid device")]
    InvalidDevice,
    #[error("the OIDC provider is unavailable: {0}")]
    Unavailable(String),
    #[error("the account cannot link this provider: {0:?}")]
    LinkRefused(LinkFailure),
    #[error("OIDC login state failed")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl From<DbErr> for OidcError {
    fn from(err: DbErr) -> Self {
        Self::Internal(err.into())
    }
}

impl From<CryptoError> for OidcError {
    fn from(err: CryptoError) -> Self {
        Self::Internal(err.into())
    }
}

/// Nothing about a flow is known, so nothing is recorded.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CallbackError {
    #[error("unknown, expired or used OIDC state")]
    InvalidState,
    /// The code is exchanged only once.
    #[error("the OIDC link flow is being verified")]
    Verifying,
    #[error("OIDC callback failed")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl From<DbErr> for CallbackError {
    fn from(err: DbErr) -> Self {
        Self::Internal(err.into())
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PollError {
    /// Indistinguishable on purpose.
    #[error("unknown, expired or used OIDC code")]
    Invalid,
    #[error("OIDC poll failed")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl From<DbErr> for PollError {
    fn from(err: DbErr) -> Self {
        Self::Internal(err.into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Denied,
    Unavailable,
    InvalidToken,
    NoLinkedAccount,
    Disabled,
    Unverified,
    ProvisioningFailed,
}

impl Failure {
    const ALL: [Self; 7] = [
        Self::Denied,
        Self::Unavailable,
        Self::InvalidToken,
        Self::NoLinkedAccount,
        Self::Disabled,
        Self::Unverified,
        Self::ProvisioningFailed,
    ];

    pub fn code(self) -> &'static str {
        LoginResult::from(self).as_str()
    }

    pub(crate) fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.code() == code)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkFailure {
    Denied,
    Unavailable,
    InvalidToken,
    IdentityTaken,
    ProviderLinked,
    /// The directory decides on LDAP accounts; an OIDC login would outlive it.
    DirectoryAccount,
    AccountInactive,
}

impl LinkFailure {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Denied => "denied",
            Self::Unavailable => "unavailable",
            Self::InvalidToken => "invalid_token",
            Self::IdentityTaken => "identity_taken",
            Self::ProviderLinked => "provider_linked",
            Self::DirectoryAccount => "directory_account",
            Self::AccountInactive => "account_inactive",
        }
    }

    pub(crate) const fn of(failure: Failure) -> Self {
        match failure {
            Failure::Denied => Self::Denied,
            Failure::InvalidToken => Self::InvalidToken,
            _ => Self::Unavailable,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IdentityError {
    #[error("identity not found")]
    NotFound,
    #[error("the account's last OIDC identity cannot be removed")]
    LastIdentity,
    #[error(transparent)]
    Refused(#[from] Refused),
    #[error("identity storage failed")]
    Database(#[source] DbErr),
}

impl From<DbErr> for IdentityError {
    fn from(err: DbErr) -> Self {
        Self::Database(err)
    }
}

impl From<GuardError> for IdentityError {
    fn from(err: GuardError) -> Self {
        match err {
            GuardError::Refused(refused) => Self::Refused(refused),
            GuardError::Database(db) => Self::Database(db),
        }
    }
}

impl From<UserError> for OidcError {
    fn from(err: UserError) -> Self {
        Self::Internal(err.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_codes_round_trip() {
        for failure in Failure::ALL {
            assert_eq!(Failure::from_code(failure.code()), Some(failure));
        }
        assert_eq!(Failure::from_code("nope"), None);
    }
}
