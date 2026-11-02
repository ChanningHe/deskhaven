//! Client contract shape (`{"error": "<msg>"}`) for errors produced by the HTTP
//! stack itself; plug into `deskhaven_http::ErrorBodies::client_api`.

use axum::Json;
use axum::response::{IntoResponse, Response};
use deskhaven_http::StackError;
use serde_json::json;

pub fn render_stack_error(err: StackError) -> Response {
    Json(json!({ "error": err.message() })).into_response()
}
