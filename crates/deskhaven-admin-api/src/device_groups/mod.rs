mod handlers;
mod view;

use utoipa_axum::routes;

use crate::{AdminRoutes, RouteError};

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
