//! Errors of the OIDC service. Messages never contain secrets (client secret, state, code,
//! verifier, tokens); upstream details stay in the source chain and the log.

use sea_orm::DbErr;

use crate::crypto::CryptoError;
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
    #[error("OIDC provider storage failed")]
    Database(#[source] DbErr),
}

impl From<DbErr> for ProviderError {
    fn from(err: DbErr) -> Self {
        Self::Database(err)
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

    /// Stored form (`oidc_flows.error`) and login event result.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Denied => "oidc_denied",
            Self::Unavailable => "oidc_unavailable",
            Self::InvalidToken => "oidc_invalid_token",
            Self::NoLinkedAccount => "no_linked_account",
            Self::Disabled => "disabled",
            Self::Unverified => "unverified",
            Self::ProvisioningFailed => "provisioning_failed",
        }
    }

    pub(crate) fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.code() == code)
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
