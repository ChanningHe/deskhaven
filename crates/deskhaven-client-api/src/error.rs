//! Client API errors: status + `{"error": "<msg>"}` (03-client-api-contract §1.3).
//! The RustDesk client shows `msg` to the user verbatim, so it must be
//! user-facing; internal detail goes only to the log (the request span carries
//! the request id).

use std::borrow::Cow;
use std::error::Error;
use std::time::Duration;

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use deskhaven_http::{INTERNAL_ERROR_BODY, error_chain};
use serde_json::json;

pub type BoxError = Box<dyn Error + Send + Sync + 'static>;

/// Body message of 429 and 503: a client translation key (`src/lang/template.rs:138`), so
/// the client shows it in the user's language. Never says why (rate limit, lockout or
/// overload) nor whether an account exists.
pub const TRY_LATER_MESSAGE: &str = "Please try later";

#[derive(Debug, thiserror::Error)]
pub enum ClientApiError {
    #[error("{0}")]
    BadRequest(Cow<'static, str>),
    #[error("{0}")]
    Unauthorized(Cow<'static, str>),
    #[error("{0}")]
    Forbidden(Cow<'static, str>),
    #[error("{0}")]
    NotFound(Cow<'static, str>),
    /// 429 + `Retry-After`: this client or account is over its limit.
    #[error("{TRY_LATER_MESSAGE}")]
    TooManyRequests { retry_after: Duration },
    /// 503 + `Retry-After`: overloaded; the request was not processed.
    #[error("{TRY_LATER_MESSAGE}")]
    Unavailable { retry_after: Duration },
    /// Detail is logged, never returned.
    #[error("{INTERNAL_ERROR_BODY}")]
    Internal(#[source] BoxError),
}

impl ClientApiError {
    pub fn internal(err: impl Into<BoxError>) -> Self {
        Self::Internal(err.into())
    }

    pub const fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::TooManyRequests { .. } => StatusCode::TOO_MANY_REQUESTS,
            Self::Unavailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    const fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::TooManyRequests { retry_after } | Self::Unavailable { retry_after } => {
                Some(*retry_after)
            }
            _ => None,
        }
    }
}

impl IntoResponse for ClientApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        match &self {
            Self::Internal(source) => {
                tracing::error!(error = %error_chain(source.as_ref()), "client api internal error");
            }
            _ => tracing::debug!(status = status.as_u16(), error = %self, "client api error"),
        }
        let mut response = (status, Json(json!({ "error": self.to_string() }))).into_response();
        if let Some(wait) = self.retry_after() {
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
