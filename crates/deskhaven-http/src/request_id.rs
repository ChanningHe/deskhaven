//! `x-request-id`: keep a well-formed incoming id, otherwise generate a UUIDv4;
//! visible to inner layers (tracing span) and echoed on every response.

use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use uuid::Uuid;

pub const X_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

/// Incoming ids longer than this are replaced (they end up in every log line).
const MAX_REQUEST_ID_LEN: usize = 64;

fn is_acceptable(value: &HeaderValue) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_REQUEST_ID_LEN
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
}

fn generate() -> HeaderValue {
    let mut buf = Uuid::encode_buffer();
    let id = Uuid::new_v4().hyphenated().encode_lower(&mut buf);
    // A hyphenated UUID is always a valid header value; the fallback is unreachable.
    HeaderValue::from_str(id).unwrap_or(HeaderValue::from_static("invalid-request-id"))
}

pub(crate) async fn propagate(mut req: Request, next: Next) -> Response {
    let id = req
        .headers()
        .get(X_REQUEST_ID)
        .filter(|v| is_acceptable(v))
        .cloned()
        .unwrap_or_else(generate);
    req.headers_mut().insert(X_REQUEST_ID, id.clone());
    let mut res = next.run(req).await;
    res.headers_mut().insert(X_REQUEST_ID, id);
    res
}
