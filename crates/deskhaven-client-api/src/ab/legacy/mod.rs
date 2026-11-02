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
//! [`MAX_LEGACY_AB_BODY`] (the stack renderer's body), 503 `Please try later` with
//! `Retry-After` when `PUSH_SLOTS` pushes are already running and none finishes within
//! `PUSH_WAIT`.
//!
//! A push is read as a stream ([`parse`]) that stops at the peer and tag caps, so a small
//! body of many tiny elements cannot grow into a large tree before the caps apply.

mod document;
mod encoding;
mod parse;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use deskhaven_core::addressbooks::{Actor, BookContents};
use deskhaven_core::state::AppState;
use deskhaven_http::DEFAULT_BODY_LIMIT;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::{Semaphore, SemaphorePermit};

use super::{ab_error, parse_body, service};
use crate::{BearerUser, ClientApiError};

/// Largest accepted legacy push (bytes): the whole book in one body, the document escaped
/// once more inside `data`. It is the stack's own request body limit
/// ([`DEFAULT_BODY_LIMIT`]): a larger route limit could never apply, the outer layer
/// answers 413 first, so the constant is derived from it rather than repeated. The default
/// `addressbook.max_peers` of 1000 peers leaves about 2 KiB per peer, several times what a
/// client writes (0.3-1 KiB); raising `max_peers` beyond about 2000 can make pushes of the
/// largest books fail with 413 (the pull still works). The new API's per-item routes keep
/// their [`super::MAX_AB_BODY`].
pub const MAX_LEGACY_AB_BODY: usize = DEFAULT_BODY_LIMIT;
/// Pushes handled at once, process-wide. A push is parsed and written in memory-hungry
/// steps (the body, the peers and the rows of a whole book), so many at once is the cost
/// that matters, not many per user.
const PUSH_SLOTS: usize = 4;
/// How long a push waits for a slot before it is shed. Pushes take milliseconds, so a
/// short wait absorbs a burst; waiting without bound would let a flood pile up buffered
/// bodies and connections. A shed legacy client pushes again on its next change.
const PUSH_WAIT: Duration = Duration::from_secs(2);
const PUSH_RETRY_AFTER: Duration = Duration::from_secs(5);
static PUSHES: Semaphore = Semaphore::const_new(PUSH_SLOTS);
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
    encoding::json_reply(body, encoding::accepts_gzip(&headers)).await
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
    encoding::json_reply(body, encoding::accepts_gzip(&headers)).await
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
    let _slot = acquire(&PUSHES, PUSH_WAIT).await?;
    let Push { data } = parse_body(&body)?;
    let books = service(&state);
    let replacement = parse::parse(&data, books.max_peers())?;
    let book = books.personal(principal.user_id).await.map_err(ab_error)?;
    books
        .replace_contents(Actor::User(principal.user_id), &book.guid, replacement)
        .await
        .map_err(ab_error)?;
    Ok(StatusCode::OK)
}

/// A slot of `slots` within `wait`, else 503 "Please try later" (the client translates it).
async fn acquire(slots: &Semaphore, wait: Duration) -> Result<SemaphorePermit<'_>, ClientApiError> {
    match tokio::time::timeout(wait, slots.acquire()).await {
        Ok(Ok(permit)) => Ok(permit),
        Ok(Err(closed)) => Err(ClientApiError::internal(closed)),
        Err(_elapsed) => Err(ClientApiError::Unavailable {
            retry_after: PUSH_RETRY_AFTER,
        }),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_push_waits_for_a_slot_then_is_shed_with_503() {
        let slots = Semaphore::new(1);
        let held = acquire(&slots, Duration::from_millis(10)).await.unwrap();

        let shed = acquire(&slots, Duration::from_millis(10))
            .await
            .unwrap_err();

        assert_eq!(shed.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(shed.to_string(), "Please try later");
        drop(held);
        let _again = acquire(&slots, Duration::from_millis(10)).await.unwrap();
    }

    #[tokio::test]
    async fn a_waiting_push_gets_the_slot_when_it_is_freed_in_time() {
        let slots = Semaphore::new(1);
        let held = acquire(&slots, Duration::from_millis(10)).await.unwrap();

        let (waiting, ()) = tokio::join!(acquire(&slots, Duration::from_secs(5)), async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            drop(held);
        });

        let _permit = waiting.unwrap();
    }

    #[test]
    fn the_route_limit_is_the_stacks_so_it_can_never_be_the_hidden_one() {
        assert_eq!(MAX_LEGACY_AB_BODY, DEFAULT_BODY_LIMIT);
    }
}
