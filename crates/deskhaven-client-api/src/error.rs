//! Client API errors: status + `{"error": "<msg>"}` (03-client-api-contract §1.3).
//! The RustDesk client shows `msg` to the user verbatim, so it must be
//! user-facing; internal detail goes only to the log (the request span carries
//! the request id).

use std::borrow::Cow;
use std::error::Error;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use deskhaven_http::{INTERNAL_ERROR_BODY, error_chain};
use serde_json::json;

pub type BoxError = Box<dyn Error + Send + Sync + 'static>;

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
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
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
        (status, Json(json!({ "error": self.to_string() }))).into_response()
    }
}
