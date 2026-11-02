//! Admin API errors: status + `{"error": {"code", "message", "details"?}}`
//! (08-web-console §6). `code` is a stable machine code the console
//! translates; internal detail goes only to the log (the request span carries
//! the request id).

use std::borrow::Cow;
use std::error::Error;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use deskhaven_http::{INTERNAL_ERROR_BODY, error_chain};
use serde::Serialize;
use serde_json::Value;

pub type BoxError = Box<dyn Error + Send + Sync + 'static>;

/// Stable wire codes. Renaming one is a breaking API change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    ValidationFailed,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    Internal,
    // Produced by the HTTP stack (see `render_stack_error`).
    BadRequest,
    MethodNotAllowed,
    PayloadTooLarge,
    Timeout,
    Unavailable,
}

impl ErrorCode {
    pub const fn status(self) -> StatusCode {
        match self {
            Self::ValidationFailed => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Conflict => StatusCode::CONFLICT,
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
            Self::BadRequest => StatusCode::BAD_REQUEST,
            Self::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            Self::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::Timeout | Self::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ValidationFailed => "validation_failed",
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::Internal => "internal",
            Self::BadRequest => "bad_request",
            Self::MethodNotAllowed => "method_not_allowed",
            Self::PayloadTooLarge => "payload_too_large",
            Self::Timeout => "timeout",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{}: {message}", code.as_str())]
pub struct AdminApiError {
    code: ErrorCode,
    message: Cow<'static, str>,
    details: Option<Value>,
    #[source]
    source: Option<BoxError>,
}

impl AdminApiError {
    pub fn new(code: ErrorCode, message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
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

#[derive(Serialize)]
struct Envelope<'a> {
    error: Body<'a>,
}

#[derive(Serialize)]
struct Body<'a> {
    code: ErrorCode,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
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
        let envelope = Envelope {
            error: Body {
                code: self.code,
                message: &self.message,
                details: self.details.as_ref(),
            },
        };
        (status, Json(envelope)).into_response()
    }
}
