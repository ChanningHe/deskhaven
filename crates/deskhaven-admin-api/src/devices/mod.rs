mod detail;
mod guard;
mod list;
mod strategy;
pub(crate) mod view;
mod write;

use utoipa_axum::routes;

pub(crate) use strategy::{EffectiveStrategyView, effective_strategy};

use crate::{AdminRoutes, RouteError};

pub(crate) fn routes(r: AdminRoutes) -> Result<AdminRoutes, RouteError> {
    r.routes(routes!(list::list_devices))?
        .routes(routes!(
            detail::get_device,
            write::update_device,
            write::delete_device
        ))?
        .routes(routes!(write::disconnect_device))
}
