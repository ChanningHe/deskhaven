mod assign;
mod deploy;
mod token_auth;

use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::post;
use deskhaven_core::api_tokens::ApiTokenPrincipal;
use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::console_audit::Actor;
use deskhaven_core::devices::Requester;
use deskhaven_core::state::AppState;

pub use assign::MAX_ASSIGN_BODY;
pub use deploy::MAX_DEPLOY_BODY;

pub(crate) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/api/devices/deploy",
            post(deploy::deploy).layer(DefaultBodyLimit::max(MAX_DEPLOY_BODY)),
        )
        .route(
            "/api/devices/cli",
            post(assign::assign).layer(DefaultBodyLimit::max(MAX_ASSIGN_BODY)),
        )
}

fn requester(principal: &ApiTokenPrincipal, client: ClientAddr) -> Requester {
    Requester {
        actor: Actor::ApiToken(principal.token_id),
        client,
    }
}
