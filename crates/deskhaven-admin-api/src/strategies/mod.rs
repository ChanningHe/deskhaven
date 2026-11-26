//! Option values reach a strategy's devices in clear (the client applies them as sent);
//! values of secret catalog options are still never returned and are masked in audit rows.

mod assignments;
mod audit;
mod catalog;
mod error;
mod handlers;
mod preview;
mod view;

use utoipa_axum::routes;

use crate::{AdminRoutes, RouteError};

type Problem = crate::ErrorEnvelope<'static>;

pub(crate) fn routes(r: AdminRoutes) -> Result<AdminRoutes, RouteError> {
    r.routes(routes!(catalog::get_catalog))?
        .routes(routes!(preview::preview_strategy))?
        .routes(routes!(
            handlers::list_strategies,
            handlers::create_strategy
        ))?
        .routes(routes!(
            handlers::get_strategy,
            handlers::update_strategy,
            handlers::delete_strategy
        ))?
        .routes(routes!(
            assignments::list_assignments,
            assignments::assign_strategy,
            assignments::unassign_strategy
        ))
}
