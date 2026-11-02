//! Admin envelope for errors produced by the HTTP stack itself; plug into
//! `deskhaven_http::ErrorBodies::admin`.

use axum::response::{IntoResponse, Response};
use deskhaven_http::StackError;

use crate::error::{AdminApiError, ErrorCode};

pub fn render_stack_error(err: StackError) -> Response {
    let code = match err {
        StackError::BadRequest => ErrorCode::BadRequest,
        StackError::NotFound => ErrorCode::NotFound,
        StackError::MethodNotAllowed => ErrorCode::MethodNotAllowed,
        StackError::PayloadTooLarge => ErrorCode::PayloadTooLarge,
        StackError::Timeout => ErrorCode::Timeout,
        StackError::Unavailable => ErrorCode::Unavailable,
        StackError::Internal => ErrorCode::Internal,
    };
    AdminApiError::new(code, err.message()).into_response()
}
