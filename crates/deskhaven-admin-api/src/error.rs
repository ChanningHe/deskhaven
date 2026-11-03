//! Admin API errors: status + `{"error": {"code", "message", "details"?}}`
//! (08-web-console §6). `code` is a stable machine code the console
//! translates; internal detail goes only to the log (the request span carries
//! the request id).

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
    // Produced by the HTTP stack (see `render_stack_error`).
    BadRequest,
    MethodNotAllowed,
    PayloadTooLarge,
    Timeout,
    Unavailable,
    // Console login and first-run setup (M3-02).
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
    // Account and group rules (06 §4.1, 05 §4.1).
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
    // Settings (05 §3.2, §3.4).
    /// The setting is fixed by the configuration file or an environment variable
    /// (`details.keys`).
    SettingPinned,
    /// The setting changed since it was read (`details.keys`, `details.conflicts`).
    VersionConflict,
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
            | Self::SettingPinned
            | Self::VersionConflict => StatusCode::CONFLICT,
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
            Self::SettingPinned => "setting_pinned",
            Self::VersionConflict => "version_conflict",
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

    /// 429 with `Retry-After` (whole seconds, rounded up, at least 1).
    pub fn rate_limited(retry_after: Duration) -> Self {
        Self {
            retry_after: Some(retry_after),
            ..Self::new(ErrorCode::RateLimited, "too many requests, try again later")
        }
    }

    /// 503 with `Retry-After` (e.g. the password hasher is saturated).
    pub fn unavailable(retry_after: Duration) -> Self {
        Self {
            retry_after: Some(retry_after),
            ..Self::new(
                ErrorCode::Unavailable,
                "temporarily unavailable, try again later",
            )
        }
    }

    /// Generic 500; `err` is logged, never returned.
    pub fn internal(err: impl Into<BoxError>) -> Self {
        Self {
            source: Some(err.into()),
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
