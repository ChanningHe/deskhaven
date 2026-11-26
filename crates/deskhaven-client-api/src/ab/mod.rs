//! `/api/ab/personal` always exists: a 404 there would switch the client to legacy mode. A
//! write's 200 has an empty body because the client shows any body as an error.

mod books;
mod legacy;
mod peers;
mod tags;

use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, post, put};
use deskhaven_core::addressbooks::{AddressBookError, AddressBookService};
use deskhaven_core::state::AppState;
use serde::de::DeserializeOwned;

pub use legacy::MAX_LEGACY_AB_BODY;

use crate::ClientApiError;

/// A peer is well under 16 KiB; a delete list of a few thousand ids fits.
pub const MAX_AB_BODY: usize = 128 * 1024;

const NOT_FOUND: &str = "Address book not found";
const FORBIDDEN: &str = "Permission denied";
/// The client's own text for a full book (`exceed_max_devices`).
const FULL: &str = "You have reached the maximum number of managed devices.";
const INVALID_BODY: &str = "Invalid request body";

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
        .merge(legacy::routes())
}

fn service(state: &AppState) -> AddressBookService {
    AddressBookService::new(
        state.db.clone(),
        state.settings.clone(),
        Arc::clone(&state.crypto),
    )
}

fn parse_body<T: DeserializeOwned>(body: &[u8]) -> Result<T, ClientApiError> {
    serde_json::from_slice(body).map_err(|_| ClientApiError::BadRequest(INVALID_BODY.into()))
}

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
