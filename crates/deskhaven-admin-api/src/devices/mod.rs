//! Device administration (M3-05): list with presence, detail (sysinfo, effective strategy
//! with sources, recent connections), owner / device group / note / status, delete and
//! disconnecting a connection. Admin routes; every write records its console audit row
//! (inside its own transaction where it writes the database).

mod detail;
mod guard;
mod list;
mod strategy;
pub(crate) mod view;
mod write;

use utoipa_axum::routes;

pub(crate) use strategy::{EffectiveStrategyView, effective_strategy};

use crate::{AdminRoutes, RouteError};

/// The `/devices` routes.
pub(crate) fn routes(r: AdminRoutes) -> Result<AdminRoutes, RouteError> {
    r.routes(routes!(list::list_devices))?
        .routes(routes!(
            detail::get_device,
            write::update_device,
            write::delete_device
        ))?
        .routes(routes!(write::disconnect_device))
}
