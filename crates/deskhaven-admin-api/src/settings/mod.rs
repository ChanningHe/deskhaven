mod audit;
mod handlers;
mod view;

pub(crate) use view::any_json;

use utoipa_axum::routes;

use crate::{AdminRoutes, RouteError};

pub(crate) fn routes(r: AdminRoutes) -> Result<AdminRoutes, RouteError> {
    r.routes(routes!(handlers::list_settings, handlers::update_settings))?
        .routes(routes!(handlers::get_setting_history))
}
