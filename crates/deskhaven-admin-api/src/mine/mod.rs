//! Every handler scopes by the session's user id; no route takes a user id from the client.

mod address_books;
mod devices;
mod sessions;

use utoipa_axum::routes;

use crate::routes::{AdminRoutes, RouteError};

pub use address_books::{BookKindView, MyAddressBook, MyAddressBooks};
pub use devices::MyDevice;
pub use sessions::{MySession, MySessionsRevoked};

pub(crate) fn routes() -> Result<AdminRoutes, RouteError> {
    AdminRoutes::new()
        .routes(routes!(sessions::list))?
        .routes(routes!(sessions::revoke))?
        .routes(routes!(sessions::revoke_others))?
        .routes(routes!(devices::list))?
        .routes(routes!(address_books::list))
}
