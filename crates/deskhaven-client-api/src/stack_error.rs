use axum::Json;
use axum::response::{IntoResponse, Response};
use deskhaven_http::StackError;
use serde_json::json;

pub fn render_stack_error(err: StackError) -> Response {
    Json(json!({ "error": err.message() })).into_response()
}
