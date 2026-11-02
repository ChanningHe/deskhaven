//! New-style address book endpoints (03-client-api-contract §5.1), read by
//! `flutter/lib/models/ab_model.dart`. Every endpoint needs a client bearer token; the rule
//! checks live in `deskhaven_core::addressbooks`.
//!
//! | case | status | body |
//! |---|---|---|
//! | read | 200 | the JSON the client parses |
//! | write done | 200 | empty (any body is shown as an error, `ab_model.dart:2002-2026`) |
//! | unknown book, or a book out of the user's reach | 404 | `{"error": "Address book not found"}` |
//! | visible book, rule too low (e.g. rule 1 writing) | 403 | `{"error": "Permission denied"}` |
//! | unknown peer / tag | 404 | `{"error": "Peer not found"}` / `"Tag not found"` |
//! | malformed input, duplicate peer / tag, book full | 400 | `{"error": ...}` |
//! | missing or invalid token | 401 | `{"error": "Unauthorized"}` (the client resets its login) |
//! | body over [`MAX_AB_BODY`] | 413 | stack renderer |
//!
//! 404 is never used for a missing endpoint the client probes here: `/api/ab/personal`
//! always exists (a 404 there would switch the client to legacy mode).

mod books;
mod peers;
mod query;
mod tags;

use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, post, put};
use deskhaven_core::addressbooks::{AddressBookError, AddressBookService};
use deskhaven_core::state::AppState;
use serde::de::DeserializeOwned;

use crate::ClientApiError;

/// Largest accepted address book request body (bytes): a peer is well under 16 KiB; a
/// delete list of a few thousand ids fits.
pub const MAX_AB_BODY: usize = 128 * 1024;

const NOT_FOUND: &str = "Address book not found";
const FORBIDDEN: &str = "Permission denied";
/// The client's own text for a full book (`src/lang/en.rs:169`, `exceed_max_devices`).
const FULL: &str = "You have reached the maximum number of managed devices.";
const INVALID_BODY: &str = "Invalid request body";

/// The §5.1 routes (merged into the client API router).
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/ab/personal", post(books::personal))
        .route("/api/ab/settings", post(books::settings))
        .route("/api/ab/shared/profiles", post(books::shared_profiles))
        .route("/api/ab/peers", post(peers::list))
        .route("/api/ab/tags/{guid}", post(tags::list))
        .route("/api/ab/peer/add/{guid}", post(peers::add))
        .route("/api/ab/peer/update/{guid}", put(peers::update))
        .route("/api/ab/peer/{guid}", delete(peers::remove))
        .route("/api/ab/tag/add/{guid}", post(tags::add))
        .route("/api/ab/tag/rename/{guid}", put(tags::rename))
        .route("/api/ab/tag/update/{guid}", put(tags::update))
        .route("/api/ab/tag/{guid}", delete(tags::remove))
        .layer(DefaultBodyLimit::max(MAX_AB_BODY))
}

/// The service over the process state (stateless: built per request).
fn service(state: &AppState) -> AddressBookService {
    AddressBookService::new(
        state.db.clone(),
        state.settings.clone(),
        Arc::clone(&state.crypto),
    )
}

/// `body` as JSON of type `T`, or 400.
fn parse_body<T: DeserializeOwned>(body: &[u8]) -> Result<T, ClientApiError> {
    serde_json::from_slice(body).map_err(|_| ClientApiError::BadRequest(INVALID_BODY.into()))
}

/// Service errors as the client shows them (see the module table).
fn ab_error(err: AddressBookError) -> ClientApiError {
    use AddressBookError as E;
    match err {
        E::NotFound => ClientApiError::NotFound(NOT_FOUND.into()),
        E::Forbidden | E::Personal => ClientApiError::Forbidden(FORBIDDEN.into()),
        E::UserNotFound => crate::auth::unauthorized(),
        E::PeerNotFound => ClientApiError::NotFound("Peer not found".into()),
        E::TagNotFound => ClientApiError::NotFound("Tag not found".into()),
        E::PeerExists => ClientApiError::BadRequest("Peer already exists".into()),
        E::TagExists => ClientApiError::BadRequest("Tag already exists".into()),
        E::Full { .. } => ClientApiError::BadRequest(FULL.into()),
        E::Invalid { field, reason } => {
            ClientApiError::BadRequest(format!("Invalid {field}: {reason}").into())
        }
        other => ClientApiError::internal(other),
    }
}
