//! `GET /client-config` (M3-11, 05 §5): what a RustDesk client needs to use this server.
//! Admin routes. Public data only: the hbbs public key, never the private one.

mod view;

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, header};
use deskhaven_config::settings::SIGNAL_RELAY_SERVERS;
use deskhaven_core::client_config::{ClientConfig, ConfigInput};
use deskhaven_core::state::AppState;
use utoipa_axum::routes;

use crate::{AdminRoutes, RouteError};
use view::ClientConfigView;

/// The `/client-config` route.
pub(crate) fn routes(r: AdminRoutes) -> Result<AdminRoutes, RouteError> {
    r.routes(routes!(get_client_config))
}

/// The server addresses, the public key and the ways to hand them to clients: the config
/// string (import box, `rustdesk --config`), its QR code, and Windows file names. Without
/// `public_url` the ID server is the host the console was opened with, flagged by
/// `host_source`; with no usable host there is no config string.
#[utoipa::path(
    get,
    path = "/client-config",
    tag = "client-config",
    operation_id = "get_client_config",
    security(("console_session" = ["admin"])),
    responses((status = 200, description = "Client configuration", body = ClientConfigView))
)]
pub async fn get_client_config(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Json<ClientConfigView> {
    let relays = state.settings.snapshot().get(&SIGNAL_RELAY_SERVERS);
    let request_host = headers.get(header::HOST).and_then(|h| h.to_str().ok());
    let config = ClientConfig::resolve(&ConfigInput {
        public_url: &state.public_url,
        request_host,
        signal_port: state.signal_port,
        relay_servers: &relays,
        public_key: &state.signal_public_key,
    });
    Json(ClientConfigView::from_config(config))
}
