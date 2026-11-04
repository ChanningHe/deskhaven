//! Shared address book administration (M3-06): books, grants (user / group x rule 1-3),
//! entries, tags and the import of devices. Admin routes; personal books are never reachable
//! here (the core service reports them as not found). Every write records its audit row in
//! its own transaction; the audit target is the book (`address_book`, its guid) and the
//! snapshots are built by the core service, which keeps entry passwords out of them.
//!
//! Entry passwords are write-only: a response carries `has_password`, never the password.
//! (A rule 1 grant does hand the password to the sharee's client, which connects with it;
//! the share dialog of the console says so, 06 §4.2.)

mod books;
mod error;
mod guard;
mod peers;
mod shares;
mod tags;
pub(crate) mod view;

use std::sync::Arc;

use deskhaven_core::addressbooks::AddressBookService;
use deskhaven_core::state::AppState;
use utoipa_axum::routes;

use crate::{AdminRoutes, RouteError};

type Problem = crate::ErrorEnvelope<'static>;

/// The address book service over the process state (stateless: built per request).
fn service(state: &AppState) -> AddressBookService {
    AddressBookService::new(
        state.db.clone(),
        state.settings.clone(),
        Arc::clone(&state.crypto),
    )
}

/// The `/address-books` routes.
pub(crate) fn routes(r: AdminRoutes) -> Result<AdminRoutes, RouteError> {
    r.routes(routes!(books::list_books, books::create_book))?
        .routes(routes!(
            books::get_book,
            books::update_book,
            books::delete_book
        ))?
        .routes(routes!(shares::list_shares, shares::set_share))?
        .routes(routes!(shares::remove_share))?
        .routes(routes!(peers::list_peers, peers::add_peer))?
        .routes(routes!(peers::update_peer))?
        .routes(routes!(peers::delete_peers))?
        .routes(routes!(peers::import_peers))?
        .routes(routes!(tags::list_tags, tags::add_tag))?
        .routes(routes!(tags::update_tag))?
        .routes(routes!(tags::delete_tags))
}
