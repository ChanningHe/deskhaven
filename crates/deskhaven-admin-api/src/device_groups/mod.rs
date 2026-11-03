//! Device group administration (M3-05): list, detail, create, update and delete. Admin
//! routes; groups from the configuration file are read-only (`managed_by_config`). Every
//! write records its audit row in its own transaction. Which devices are in a group is set
//! on the device (`PATCH /devices/{id}`).

mod handlers;
mod view;

use utoipa_axum::routes;

use crate::{AdminRoutes, RouteError};

/// The `/device-groups` routes.
pub(crate) fn routes(r: AdminRoutes) -> Result<AdminRoutes, RouteError> {
    r.routes(routes!(
        handlers::list_device_groups,
        handlers::create_device_group
    ))?
    .routes(routes!(
        handlers::get_device_group,
        handlers::update_device_group,
        handlers::delete_device_group
    ))
}
