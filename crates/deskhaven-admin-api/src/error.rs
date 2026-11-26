//! `code` is a stable machine code the console translates; internal detail goes only to the
//! log.

use std::borrow::Cow;
use std::error::Error;
use std::time::Duration;

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use deskhaven_http::{INTERNAL_ERROR_BODY, error_chain};
use serde::Serialize;
use serde_json::Value;
use utoipa::ToSchema;

pub type BoxError = Box<dyn Error + Send + Sync + 'static>;

/// A deadlock victim's peer transaction is long done by then.
const TRANSIENT_RETRY_AFTER: Duration = Duration::from_secs(1);

/// Stable wire codes. Renaming one is a breaking API change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    ValidationFailed,
    Unauthorized,
    Forbidden,
    /// A state-changing request without the matching `X-Deskhaven-CSRF` header.
    CsrfFailed,
    NotFound,
    Conflict,
    RateLimited,
    UnsupportedMediaType,
    Internal,
    BadRequest,
    MethodNotAllowed,
    PayloadTooLarge,
    Timeout,
    Unavailable,
    /// Login: wrong username or password, or an account without a local password; one
    /// code for all, so it does not tell whether the account exists.
    InvalidCredentials,
    /// Login: right password, disabled account.
    AccountDisabled,
    /// Login: right password, unverified account.
    AccountUnverified,
    /// A wrong, replayed or used TOTP / recovery code (the step can be retried).
    InvalidCode,
    /// Login second step: the challenge expired, was used up or is unknown; log in again.
    InvalidChallenge,
    /// Password change: the current password is wrong.
    WrongPassword,
    /// The console is reached over plain HTTP: browsers would drop its `Secure` cookies.
    InsecureTransport,
    /// The session must finish its account setup first (`details.step`).
    SetupRequired,
    /// First-run setup: unknown, expired or used setup link.
    InvalidSetupToken,
    /// First-run setup: the instance already has an owner.
    SetupCompleted,
    /// A console OIDC login failed (`details.reason`).
    OidcFailed,
    /// Only an owner may create, change or delete an owner, or promote to owner.
    OwnerRequired,
    /// The change would leave no active owner.
    LastOwner,
    /// Deleting one's own account.
    SelfDelete,
    /// An action on one's own account that belongs on the account page (`/me`), behind
    /// re-authentication (resetting one's own 2FA).
    SelfService,
    /// The resource is defined in the configuration file and read-only in the console.
    ManagedByConfig,
    /// The account has no local password or local second factor (OIDC sign-in; LDAP and
    /// OIDC accounts for a password reset).
    NotLocalAccount,
    /// `auth.tfa.enforce` requires a second factor of this account: it cannot be turned off.
    TfaEnforced,
    /// The only OIDC identity of an account that signs in through OIDC alone (M4-09).
    LastIdentity,
    /// The setting is fixed by the configuration file or an environment variable
    /// (`details.keys`).
    SettingPinned,
    /// The setting changed since it was read (`details.keys`, `details.conflicts`).
    VersionConflict,
    /// `web_client.enabled` is off.
    WebClientDisabled,
    /// The device has not been seen by the signal server for 30 s.
    DeviceOffline,
    /// The device did not answer the punch in time.
    DeviceNoAnswer,
    /// The device relays through a relay server of its own (`details.relay_server`), which
    /// the browser cannot reach through this deployment.
    ForeignRelay,
    /// No relay server is configured (`signal.relay_servers`).
    NoRelay,
    /// The WebRTC offer is not an offer endpoint within the size limit.
    InvalidOffer,
    /// An ICE candidate the signal server refuses (empty or too long).
    InvalidCandidate,
    /// The session's route is closed (expired, no WebRTC session, or closed).
    SessionClosed,
    /// The session already passed the most ICE candidates to the device.
    TooManyCandidates,
    /// The user holds the most web client sessions at once.
    TooManySessions,
}

impl ErrorCode {
    pub const fn status(self) -> StatusCode {
        match self {
            Self::ValidationFailed => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden | Self::CsrfFailed => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Conflict => StatusCode::CONFLICT,
            Self::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            Self::UnsupportedMediaType => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
            Self::BadRequest => StatusCode::BAD_REQUEST,
            Self::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            Self::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::Timeout | Self::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            // Before any session exists: 401 cannot log anyone out.
            Self::InvalidCredentials | Self::InvalidChallenge => StatusCode::UNAUTHORIZED,
            // With or without a session, never 401 (the console treats 401 as logged out).
            Self::InvalidCode | Self::WrongPassword => StatusCode::UNPROCESSABLE_ENTITY,
            Self::AccountDisabled
            | Self::AccountUnverified
            | Self::InsecureTransport
            | Self::SetupRequired
            | Self::InvalidSetupToken
            | Self::OidcFailed
            | Self::OwnerRequired => StatusCode::FORBIDDEN,
            Self::SetupCompleted
            | Self::LastOwner
            | Self::SelfDelete
            | Self::SelfService
            | Self::ManagedByConfig
            | Self::NotLocalAccount
            | Self::TfaEnforced
            | Self::LastIdentity
            | Self::SettingPinned
            | Self::VersionConflict => StatusCode::CONFLICT,
            Self::WebClientDisabled => StatusCode::FORBIDDEN,
            Self::DeviceOffline | Self::ForeignRelay | Self::NoRelay => StatusCode::CONFLICT,
            Self::DeviceNoAnswer => StatusCode::GATEWAY_TIMEOUT,
            Self::InvalidOffer | Self::InvalidCandidate => StatusCode::BAD_REQUEST,
            Self::SessionClosed => StatusCode::GONE,
            Self::TooManyCandidates | Self::TooManySessions => StatusCode::TOO_MANY_REQUESTS,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ValidationFailed => "validation_failed",
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::CsrfFailed => "csrf_failed",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::RateLimited => "rate_limited",
            Self::UnsupportedMediaType => "unsupported_media_type",
            Self::Internal => "internal",
            Self::BadRequest => "bad_request",
            Self::MethodNotAllowed => "method_not_allowed",
            Self::PayloadTooLarge => "payload_too_large",
            Self::Timeout => "timeout",
            Self::Unavailable => "unavailable",
            Self::InvalidCredentials => "invalid_credentials",
            Self::AccountDisabled => "account_disabled",
            Self::AccountUnverified => "account_unverified",
            Self::InvalidCode => "invalid_code",
            Self::InvalidChallenge => "invalid_challenge",
            Self::WrongPassword => "wrong_password",
            Self::InsecureTransport => "insecure_transport",
            Self::SetupRequired => "setup_required",
            Self::InvalidSetupToken => "invalid_setup_token",
            Self::SetupCompleted => "setup_completed",
            Self::OidcFailed => "oidc_failed",
            Self::OwnerRequired => "owner_required",
            Self::LastOwner => "last_owner",
            Self::SelfDelete => "self_delete",
            Self::SelfService => "self_service",
            Self::ManagedByConfig => "managed_by_config",
            Self::NotLocalAccount => "not_local_account",
            Self::TfaEnforced => "tfa_enforced",
            Self::LastIdentity => "last_identity",
            Self::SettingPinned => "setting_pinned",
            Self::VersionConflict => "version_conflict",
            Self::WebClientDisabled => "web_client_disabled",
            Self::DeviceOffline => "device_offline",
            Self::DeviceNoAnswer => "device_no_answer",
            Self::ForeignRelay => "foreign_relay",
            Self::NoRelay => "no_relay",
            Self::InvalidOffer => "invalid_offer",
            Self::InvalidCandidate => "invalid_candidate",
            Self::SessionClosed => "session_closed",
            Self::TooManyCandidates => "too_many_candidates",
            Self::TooManySessions => "too_many_sessions",
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{}: {message}", code.as_str())]
pub struct AdminApiError {
    code: ErrorCode,
    message: Cow<'static, str>,
    details: Option<Value>,
    retry_after: Option<Duration>,
    #[source]
    source: Option<BoxError>,
}

impl AdminApiError {
    pub fn new(code: ErrorCode, message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
            retry_after: None,
            source: None,
        }
    }

    #[must_use]
    pub fn with_details(self, details: Value) -> Self {
        Self {
            details: Some(details),
            ..self
        }
    }

    pub fn validation_failed(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ErrorCode::ValidationFailed, message)
    }

    pub fn not_found(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    pub fn conflict(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ErrorCode::Conflict, message)
    }

    pub fn unauthorized() -> Self {
        Self::new(ErrorCode::Unauthorized, "authentication required")
    }

    pub fn forbidden() -> Self {
        Self::new(ErrorCode::Forbidden, "permission denied")
    }

    pub fn csrf_failed() -> Self {
        Self::new(ErrorCode::CsrfFailed, "missing or invalid CSRF token")
    }

    pub fn rate_limited(retry_after: Duration) -> Self {
        Self {
            retry_after: Some(retry_after),
            ..Self::new(ErrorCode::RateLimited, "too many requests, try again later")
        }
    }

    pub fn unavailable(retry_after: Duration) -> Self {
        Self {
            retry_after: Some(retry_after),
            ..Self::new(
                ErrorCode::Unavailable,
                "temporarily unavailable, try again later",
            )
        }
    }

    /// `err` is logged, never returned. A deadlock or serialization failure in its chain is a
    /// 503 with `Retry-After` instead: nothing was written and a retry can succeed.
    pub fn internal(err: impl Into<BoxError>) -> Self {
        let source: BoxError = err.into();
        if deskhaven_db::chain_has_transient_conflict(source.as_ref()) {
            return Self {
                source: Some(source),
                ..Self::unavailable(TRANSIENT_RETRY_AFTER)
            };
        }
        Self {
            source: Some(source),
            ..Self::new(ErrorCode::Internal, INTERNAL_ERROR_BODY)
        }
    }

    pub const fn code(&self) -> ErrorCode {
        self.code
    }
}

/// The error body of every admin API failure (08-web-console §6).
#[derive(Serialize, ToSchema)]
pub struct ErrorEnvelope<'a> {
    error: ErrorBody<'a>,
}

#[derive(Serialize, ToSchema)]
pub struct ErrorBody<'a> {
    code: ErrorCode,
    /// Human-readable, English; the console translates by `code`.
    message: &'a str,
    /// Machine-readable context, e.g. `{"fields": {"<path>": ["<problem>"]}}` for
    /// `validation_failed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    details: Option<&'a Value>,
}

impl IntoResponse for AdminApiError {
    fn into_response(self) -> Response {
        let status = self.code.status();
        match &self.source {
            Some(source) => {
                tracing::error!(code = self.code.as_str(), error = %error_chain(source.as_ref()), "admin api error");
            }
            None => {
                tracing::debug!(code = self.code.as_str(), message = %self.message, "admin api error")
            }
        }
        let envelope = ErrorEnvelope {
            error: ErrorBody {
                code: self.code,
                message: &self.message,
                details: self.details.as_ref(),
            },
        };
        let mut response = (status, Json(envelope)).into_response();
        if let Some(wait) = self.retry_after {
            response.headers_mut().insert(
                header::RETRY_AFTER,
                HeaderValue::from(retry_after_secs(wait)),
            );
        }
        response
    }
}

/// Whole seconds, rounded up, at least 1 (`Retry-After: 0` would invite a tight loop).
fn retry_after_secs(wait: Duration) -> u64 {
    let rounded_up = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
    rounded_up.max(1)
}

#[cfg(test)]
mod tests {
    use axum::response::IntoResponse;
    use deskhaven_core::addressbooks::AddressBookError;
    use deskhaven_db::coded_error::coded_error;

    use super::*;

    fn status_and_retry(err: AdminApiError) -> (StatusCode, Option<String>) {
        let response = err.into_response();
        let retry = response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        (response.status(), retry)
    }

    #[test]
    fn a_deadlock_victim_is_a_retryable_503_whatever_service_wrapped_it() {
        for code in ["40P01", "40001"] {
            let wrapped = AddressBookError::Database(coded_error(code));
            let got = status_and_retry(AdminApiError::from(wrapped));
            assert_eq!(
                got,
                (StatusCode::SERVICE_UNAVAILABLE, Some("1".to_owned())),
                "{code}"
            );
        }
    }

    #[test]
    fn other_database_failures_stay_500_without_retry_after() {
        let wrapped = AddressBookError::Database(coded_error("23505"));
        let got = status_and_retry(AdminApiError::from(wrapped));
        assert_eq!(got, (StatusCode::INTERNAL_SERVER_ERROR, None));
    }
}
