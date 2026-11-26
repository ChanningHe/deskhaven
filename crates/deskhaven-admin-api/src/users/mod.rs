mod actions;
mod crud;
mod csv;
mod identities;
mod import;
pub(crate) mod list;
mod rules;
pub(crate) mod view;

use utoipa_axum::routes;

pub use import::{MAX_CSV_BYTES, MAX_IMPORT_ROWS};
pub(crate) use rules::EMAIL_CHANGED;

use crate::{AdminRoutes, RouteError};

pub(crate) fn routes(r: AdminRoutes) -> Result<AdminRoutes, RouteError> {
    r.routes(routes!(list::list_users, crud::create_user))?
        .routes(routes!(import::import_users))?
        .routes(routes!(
            crud::get_user,
            crud::update_user,
            crud::delete_user
        ))?
        .routes(routes!(actions::disable_user))?
        .routes(routes!(actions::enable_user))?
        .routes(routes!(actions::reset_password))?
        .routes(routes!(actions::reset_2fa))?
        .routes(routes!(actions::revoke_sessions))?
        .routes(routes!(actions::unlock))?
        .routes(routes!(identities::unlink_user_identity))
}
