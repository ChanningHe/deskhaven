mod handlers;
mod rules;
mod view;

use utoipa_axum::routes;

use crate::{AdminRoutes, RouteError};

pub(crate) fn routes(r: AdminRoutes) -> Result<AdminRoutes, RouteError> {
    r.routes(routes!(
        handlers::list_api_tokens,
        handlers::create_api_token
    ))?
    .routes(routes!(handlers::revoke_api_token))
}
