mod guard;
mod handlers;
mod members;
mod view;

use utoipa_axum::routes;

pub(crate) use view::ManagedByView;

use crate::users::list;
use crate::{AdminRoutes, RouteError};

pub(crate) fn routes(r: AdminRoutes) -> Result<AdminRoutes, RouteError> {
    r.routes(routes!(handlers::list_groups, handlers::create_group))?
        .routes(routes!(
            handlers::get_group,
            handlers::update_group,
            handlers::delete_group
        ))?
        .routes(routes!(list::list_group_members, members::add_member))?
        .routes(routes!(members::remove_member))
}
