//! Strategy administration (M3-07): the client option catalog, strategies (CRUD with
//! option merge patches), their assignments to targets, and the preview of a device's
//! effective strategy. Admin routes; strategies from the configuration file are read-only
//! (`managed_by_config`). Every write records its audit row in its own transaction.
//!
//! Option values reach the devices of a strategy in clear (the client applies them as
//! sent). Values of secret catalog options are still never returned by this API and are
//! masked in audit rows.

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

/// The `/strategies`, `/strategy-catalog` and `/strategy-preview` routes.
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
