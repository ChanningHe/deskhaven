//! `rustdesk --deploy` and `rustdesk --assign` (03 §2.8, §2.9): device placement by an API
//! token (`deskhaven_core::api_tokens`), not by the device itself. Every change is written
//! to the console audit log as the token.

mod assign;
mod deploy;
mod token_auth;

use std::net::IpAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::post;
use deskhaven_core::api_tokens::ApiTokenPrincipal;
use deskhaven_core::console_audit::{Actor, NewConsoleAudit};
use deskhaven_core::state::AppState;
use serde_json::Value;

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

/// Records a committed change of device `rustdesk_id` made through `principal`'s token. A
/// failure is logged: the change itself already happened.
async fn audit(
    state: &AppState,
    principal: &ApiTokenPrincipal,
    action: &str,
    rustdesk_id: &str,
    after: Value,
    ip: IpAddr,
) {
    let entry = NewConsoleAudit {
        actor: Actor::ApiToken(principal.token_id),
        action: action.to_owned(),
        target_type: "device".to_owned(),
        target_id: rustdesk_id.to_owned(),
        before: None,
        after: Some(after),
        ip: Some(ip),
    };
    if let Err(err) = state.console_audit.record(&entry).await {
        tracing::error!(action, rustdesk_id, error = %err, "cannot audit a device change");
    }
}
