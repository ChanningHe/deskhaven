//! Errors of the OIDC service. Messages never contain secrets (client secret, state, code,
//! verifier, tokens); upstream details stay in the source chain and the log.

use sea_orm::DbErr;

use crate::console_audit::{GuardError, Refused};
use crate::crypto::CryptoError;
use crate::login_events::LoginResult;
use crate::users::UserError;

/// Why a provider operation failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ProviderError {
    #[error("OIDC provider not found")]
    NotFound,
    #[error("an OIDC provider named like this already exists")]
    Conflict,
    #[error("invalid {field}: {reason}")]
    Invalid { field: &'static str, reason: String },
    /// Declared in the bootstrap configuration: read-only here (05 §4.1).
    #[error("the OIDC provider is managed by the configuration file")]
    ManagedByConfig,
    /// Accounts are linked to it (`user_identities`): disable it, or unlink them first.
    #[error("the OIDC provider still has linked accounts")]
    InUse,
    #[error("sealing the client secret failed")]
    Crypto(#[source] CryptoError),
    /// A [`TxGuard`](crate::console_audit::TxGuard) declined the change (nothing was written).
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

/// Why a login could not start.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OidcError {
    /// No `public_url`, so there is no callback URL (06 §3.4).
    #[error("OIDC login is not configured")]
    NotConfigured,
    /// Unknown or disabled provider name.
    #[error("unknown OIDC provider")]
    UnknownProvider,
    /// The device fields are not acceptable for a client session.
    #[error("invalid device")]
    InvalidDevice,
    /// Discovery failed or returned unusable endpoints.
    #[error("the OIDC provider is unavailable: {0}")]
    Unavailable(String),
    /// A link flow cannot start for this account (M4-09).
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

/// Why a callback was not processed at all: nothing about a flow is known, so nothing is
/// recorded.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CallbackError {
    /// Unknown, expired or already used `state` (or no state).
    #[error("unknown, expired or used OIDC state")]
    InvalidState,
    /// Another request is verifying this link flow right now: ask again shortly (the code
    /// is exchanged only once).
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

/// Why a poll does not answer with a login or a flow outcome.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PollError {
    /// Unknown, expired or already handed-out code, or a device other than the one the flow
    /// was started from. Indistinguishable on purpose.
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

/// How a claimed flow failed. Stored in `oidc_flows.error` by [`code`](Self::code) and
/// reported to the poller; each has a login event result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The IdP returned an error (user cancelled, consent denied) or no code.
    Denied,
    /// Discovery or the token request failed, or the provider was disabled meanwhile.
    Unavailable,
    /// The token response or ID token failed verification (signature, issuer, audience,
    /// expiry, nonce, missing ID token or subject).
    InvalidToken,
    /// No linked account and none may be linked or created (06 §3.4).
    NoLinkedAccount,
    Disabled,
    Unverified,
    /// Creating the account failed (no usable username, or a concurrent conflict).
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

    /// Stored form (`oidc_flows.error`): the login event result of the failure.
    pub fn code(self) -> &'static str {
        LoginResult::from(self).as_str()
    }

    pub(crate) fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.code() == code)
    }
}

/// Why a link flow did not link (M4-09). Nothing was written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkFailure {
    /// The IdP returned an error (user cancelled, consent denied) or no code.
    Denied,
    /// Discovery or the token request failed, or the provider was disabled meanwhile.
    Unavailable,
    /// The token response or ID token failed verification.
    InvalidToken,
    /// The identity is linked to another account.
    IdentityTaken,
    /// The account already has (another) identity at this provider.
    ProviderLinked,
    /// LDAP accounts: the directory decides on them, an OIDC login would outlive it.
    DirectoryAccount,
    /// The account is gone or no longer active.
    AccountInactive,
}

impl LinkFailure {
    /// Stable machine name (the admin API's `details.reason`).
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

    /// The link outcome of a failed token verification step.
    pub(crate) const fn of(failure: Failure) -> Self {
        match failure {
            Failure::Denied => Self::Denied,
            Failure::InvalidToken => Self::InvalidToken,
            _ => Self::Unavailable,
        }
    }
}

/// Why listing or removing an identity failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IdentityError {
    /// No such identity (or not one of the given account's).
    #[error("identity not found")]
    NotFound,
    /// The only identity of an account that signs in through OIDC alone.
    #[error("the account's last OIDC identity cannot be removed")]
    LastIdentity,
    /// The caller's [`TxGuard`](crate::console_audit::TxGuard) declined the change.
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
