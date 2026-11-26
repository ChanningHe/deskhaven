//! A projection of the user's personal book: nothing is stored apart from the new API's rows.
//! `licensed_devices` is always 0: a positive value makes older clients gzip their pushes and
//! stop at that many peers, while deskhaven enforces its cap on the server. A push is read as
//! a stream that stops at the caps, so a small body of tiny elements cannot grow into a
//! large tree first.

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

/// The stack's own body limit: a larger route limit could never apply. The default
/// `addressbook.max_peers` of 1000 leaves about 2 KiB per peer; beyond about 2000 peers the
/// largest pushes can fail with 413 (the pull still works).
pub const MAX_LEGACY_AB_BODY: usize = DEFAULT_BODY_LIMIT;
/// Process-wide: a push is parsed and written in memory-hungry steps, so many at once is the
/// cost that matters, not many per user.
const PUSH_SLOTS: usize = 4;
/// Pushes take milliseconds, so a short wait absorbs a burst; an unbounded wait would let a
/// flood pile up buffered bodies. A shed client pushes again on its next change.
const PUSH_WAIT: Duration = Duration::from_secs(2);
const PUSH_RETRY_AFTER: Duration = Duration::from_secs(5);
static PUSHES: Semaphore = Semaphore::const_new(PUSH_SLOTS);
const UNSUPPORTED_ENCODING: &str = "Unsupported content encoding";
const LICENSED_DEVICES: u32 = 0;

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

/// The request body is ignored.
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

/// 503 "Please try later": the client translates that text.
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
            drop(held);
        });

        let _permit = waiting.unwrap();
    }
}
