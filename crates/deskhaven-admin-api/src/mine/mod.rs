//! What the signed-in user sees of their own data (M3-13, `self` permission): sessions,
//! devices and address books. Every handler scopes by the session's user id; no route takes
//! a user id from the client, and the session and address book services match ownership in
//! the statement itself.
//!
//! | method | path | what | audit |
//! |---|---|---|---|
//! | `GET` | `/me/sessions` | client and console sessions, the current one marked | |
//! | `DELETE` | `/me/sessions/{id}` | revoke one (the current one signs out) | `session.revoke` |
//! | `POST` | `/me/sessions/revoke-others` | revoke all but the current | `session.revoke_others` |
//! | `GET` | `/me/devices` | owned devices with online state (paged) | |
//! | `GET` | `/me/address-books` | personal and shared books with the user's rule | |

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
