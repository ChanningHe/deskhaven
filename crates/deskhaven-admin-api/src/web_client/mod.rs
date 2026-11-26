//! Audit rows never record offers, answers, candidates or ICE credentials; a session answers
//! only the user who opened it (anyone else: 404).

mod error;
mod handlers;
mod view;

use utoipa_axum::routes;

use crate::routes::{AdminRoutes, RouteError};

pub(crate) fn routes(routes: AdminRoutes) -> Result<AdminRoutes, RouteError> {
    routes
        .routes(routes!(handlers::open))?
        .routes(routes!(handlers::add_candidate, handlers::poll_candidates))?
        .routes(routes!(handlers::close))
}
