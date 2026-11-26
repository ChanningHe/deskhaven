//! Entry passwords are write-only: a response carries `has_password`, never the password.

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

fn service(state: &AppState) -> AddressBookService {
    AddressBookService::new(
        state.db.clone(),
        state.settings.clone(),
        Arc::clone(&state.crypto),
    )
}

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
