//! Legacy address book endpoints (03 §5.2), for clients that hold the book as one document:
//! the Sciter UI (always) and any client whose `POST /api/ab/personal` probe fails (it never
//! does here, see [`super`]). They are a projection of the user's **personal** book (no
//! name lookup, no shared books): nothing is stored apart from the new API's rows.
//!
//! | endpoint | request | reply |
//! |---|---|---|
//! | `GET /api/ab` | `Accept-Encoding: gzip` | `{"data": "<document>", "licensed_devices": 0}`, or the literal `null` for an empty book; gzip when accepted |
//! | `POST /api/ab/get` | any body (the Sciter UI sends `{}`) | `{"data": "<document>", "updated_at": <unix ms>}`, or `null`; same negotiation |
//! | `POST /api/ab` | `{"data": "<document>"}`, never compressed | 200, empty body |
//!
//! `<document>` is described in [`document`]. A push replaces the personal book's peers and
//! tags in one transaction ([`deskhaven_core::addressbooks::AddressBookService::replace_contents`]):
//! same validation and `addressbook.max_peers` cap as the per-item API, fields a legacy
//! client does not send survive, `password` is dropped, the hash is sealed.
//!
//! `licensed_devices` is always 0. A positive value makes clients before 2026-01 gzip their
//! pushes and stop at that many peers; deskhaven enforces its cap on the server and says so
//! (400, the client's own "maximum number of managed devices" text), so gzip bodies need not
//! be decoded (and are refused: 400 "Unsupported content encoding").
//!
//! Errors are the new API's ([`super::ab_error`]): 401 without a credential, 400
//! `{"error": ...}` for a malformed or over-cap push (nothing changes), 413 over
//! [`MAX_LEGACY_AB_BODY`] (the stack renderer's body).

mod document;
mod encoding;

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use deskhaven_core::addressbooks::{Actor, BookContents};
use deskhaven_core::state::AppState;
use serde::Deserialize;
use serde_json::json;

use super::{ab_error, parse_body, service};
use crate::{BearerUser, ClientApiError};

/// Largest accepted legacy push (bytes): the whole book in one body, the document escaped
/// once more inside `data`. The default `addressbook.max_peers` of 1000 peers of a typical
/// 0.3-1 KiB each is 0.3-1 MiB, so 4 MiB leaves 4 KiB per peer; raising `max_peers` far
/// beyond 4000 makes pushes of the largest books fail with 413 (the pull still works).
/// The new API's per-item routes keep their [`super::MAX_AB_BODY`].
pub const MAX_LEGACY_AB_BODY: usize = 4 * 1024 * 1024;
const UNSUPPORTED_ENCODING: &str = "Unsupported content encoding";
/// Sent as `licensed_devices` (see the module docs).
const LICENSED_DEVICES: u32 = 0;

/// The legacy routes, with their own body limit.
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/ab", get(pull).post(push))
        .route("/api/ab/get", post(pull_sciter))
        .layer(DefaultBodyLimit::max(MAX_LEGACY_AB_BODY))
}

#[derive(Deserialize)]
struct Push {
    data: String,
}

/// `GET /api/ab`.
async fn pull(
    BearerUser(principal): BearerUser,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, ClientApiError> {
    let contents = personal_contents(&state, principal.user_id).await?;
    let body = reply(
        &contents,
        |data| json!({ "data": data, "licensed_devices": LICENSED_DEVICES }),
    );
    encoding::json_reply(body, encoding::accepts_gzip(&headers))
}

/// `POST /api/ab/get` (Sciter UI); the request body is ignored.
async fn pull_sciter(
    BearerUser(principal): BearerUser,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, ClientApiError> {
    let contents = personal_contents(&state, principal.user_id).await?;
    let updated_at = contents.updated_at.timestamp_millis();
    let body = reply(
        &contents,
        |data| json!({ "data": data, "updated_at": updated_at }),
    );
    encoding::json_reply(body, encoding::accepts_gzip(&headers))
}

/// `POST /api/ab`: replaces the personal book's peers and tags.
async fn push(
    BearerUser(principal): BearerUser,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ClientApiError> {
    if !encoding::is_plain(&headers) {
        return Err(ClientApiError::BadRequest(UNSUPPORTED_ENCODING.into()));
    }
    let Push { data } = parse_body(&body)?;
    let replacement = document::parse(&data)?;
    let books = service(&state);
    let book = books.personal(principal.user_id).await.map_err(ab_error)?;
    books
        .replace_contents(Actor::User(principal.user_id), &book.guid, replacement)
        .await
        .map_err(ab_error)?;
    Ok(StatusCode::OK)
}

async fn personal_contents(state: &AppState, user_id: i64) -> Result<BookContents, ClientApiError> {
    let books = service(state);
    let book = books.personal(user_id).await.map_err(ab_error)?;
    books
        .contents(Actor::User(user_id), &book.guid)
        .await
        .map_err(ab_error)
}

/// The reply body: `null` for an empty book, else `envelope(<document>)`.
fn reply(contents: &BookContents, envelope: impl FnOnce(String) -> serde_json::Value) -> String {
    if contents.peers.is_empty() && contents.tags.is_empty() {
        return "null".to_owned();
    }
    envelope(document::render(contents)).to_string()
}
