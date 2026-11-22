//! `GET /update-check`: whether a newer deskhaven release is out, for the version footnote
//! of the console. Answers from the server's cache at once (see `deskhaven_core::update`);
//! admins only, so the console does not tell anyone without a role that the server is behind.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use deskhaven_core::state::AppState;
use deskhaven_core::update::AvailableUpdate;
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::routes;

use crate::{AdminRoutes, RouteError};

/// A newer release than the running server, if one is known. Both fields are null, or
/// neither: null while `updates.check` is off, before the first check finished, or when the
/// server is current.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct UpdateCheckView {
    /// The release's version, digits and dots only (e.g. `0.1.3`).
    pub latest: Option<String>,
    /// Its release page in the source repository (https).
    pub release_url: Option<String>,
}

impl From<Option<AvailableUpdate>> for UpdateCheckView {
    fn from(update: Option<AvailableUpdate>) -> Self {
        match update {
            Some(update) => Self {
                latest: Some(update.version.to_string()),
                release_url: Some(update.release_url),
            },
            None => Self {
                latest: None,
                release_url: None,
            },
        }
    }
}

/// The newer release the server knows of. Never waits for GitHub: a stale or empty cache
/// starts a check in the background, which a later request sees.
#[utoipa::path(
    get,
    path = "/update-check",
    tag = "update-check",
    operation_id = "get_update_check",
    security(("console_session" = ["admin"])),
    responses((status = 200, description = "The newer release, if any", body = UpdateCheckView))
)]
pub async fn get_update_check(State(state): State<Arc<AppState>>) -> Json<UpdateCheckView> {
    Json(state.updates.available().into())
}

/// The `/update-check` route.
pub(crate) fn routes(r: AdminRoutes) -> Result<AdminRoutes, RouteError> {
    r.routes(routes!(get_update_check))
}
