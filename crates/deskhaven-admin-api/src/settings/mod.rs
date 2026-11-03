//! Runtime settings (M3-10, 05 §3): list with source and pin state, batch update with
//! optimistic versions, reset to the default, per-key history. Owner only. Every write
//! records its audit row in the settings transaction (secrets as `{"set": bool}`).

mod audit;
mod handlers;
mod view;

use utoipa_axum::routes;

use crate::{AdminRoutes, RouteError};

/// The `/settings` routes.
pub(crate) fn routes(r: AdminRoutes) -> Result<AdminRoutes, RouteError> {
    r.routes(routes!(handlers::list_settings, handlers::update_settings))?
        .routes(routes!(handlers::get_setting_history))
}
