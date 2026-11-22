//! Web client sessions (remote control from the console; `self` permission): the server
//! punches the device on the browser's behalf (`deskhaven_core::web_client`) and relays the
//! WebRTC session's ICE candidates. Off unless `web_client.enabled`; a session answers only
//! the user who opened it (anyone else: 404).
//!
//! | method | path | what | audit |
//! |---|---|---|---|
//! | `POST` | `/web-client/sessions` | open: relay parameters, WebRTC answer, ICE servers | `web_client.session.open`, every outcome |
//! | `POST` | `/web-client/sessions/{id}/candidates` | a browser ICE candidate to the device | |
//! | `GET` | `/web-client/sessions/{id}/candidates?after=&wait=` | device ICE candidates (long poll) | |
//! | `DELETE` | `/web-client/sessions/{id}` | close, revoking the route | `web_client.session.close` |
//!
//! Audit rows record the device, the source IP, whether an offer was sent and the outcome;
//! never offers, answers, candidates or ICE credentials.

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
