//! API token administration (M3-12): list, create (the secret is returned once) and revoke.
//! Admin routes; every write records its audit row in its own transaction, without the
//! secret. Who may revoke whose token: see `rules`.

mod handlers;
mod rules;
mod view;

use utoipa_axum::routes;

use crate::{AdminRoutes, RouteError};

/// The `/api-tokens` routes.
pub(crate) fn routes(r: AdminRoutes) -> Result<AdminRoutes, RouteError> {
    r.routes(routes!(
        handlers::list_api_tokens,
        handlers::create_api_token
    ))?
    .routes(routes!(handlers::revoke_api_token))
}
