//! HTTP API consumed by the official RustDesk client (`/api/*`).
//!
//! Every endpoint is contract-tested by fixtures in `tests/contract`
//! (crate `deskhaven-contract`, which this crate must never depend on).

mod ab;
mod account;
mod audit;
mod auth;
mod client;
mod devices;
mod error;
mod heartbeat;
mod login;
mod login_options;
mod oidc;
mod stack_error;
mod sysinfo;
mod user_payload;
mod version;

use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use deskhaven_core::state::AppState;

pub use ab::{MAX_AB_BODY, MAX_LEGACY_AB_BODY};
pub use audit::MAX_AUDIT_BODY;
pub use auth::{BearerUser, UNAUTHORIZED_MESSAGE};
pub use error::{BoxError, ClientApiError, TRY_LATER_MESSAGE};
pub use heartbeat::MAX_HEARTBEAT_BODY;
pub use login::MAX_LOGIN_BODY;
pub use oidc::{
    INVALID_LOGIN_MESSAGE as OIDC_INVALID_LOGIN_MESSAGE, PENDING_MESSAGE as OIDC_PENDING_MESSAGE,
};
pub use stack_error::render_stack_error;
pub use sysinfo::MAX_SYSINFO_BODY;
pub use version::{SOURCE_URL, VERSION};

/// Client API routes, without middleware or state: the server applies the
/// stack and provides the state (`with_state`) when composing. Handlers that
/// need services take `State<Arc<AppState>>`.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/login-options", get(login_options::login_options))
        .route(
            "/api/login",
            post(login::login).layer(DefaultBodyLimit::max(login::MAX_LOGIN_BODY)),
        )
        .route("/api/currentUser", post(account::current_user))
        .route("/api/logout", post(account::logout))
        .route("/api/version", get(version::version))
        .route(
            "/api/sysinfo",
            post(sysinfo::sysinfo).layer(DefaultBodyLimit::max(sysinfo::MAX_SYSINFO_BODY)),
        )
        .route(
            "/api/heartbeat",
            post(heartbeat::heartbeat).layer(DefaultBodyLimit::max(MAX_HEARTBEAT_BODY)),
        )
        .merge(oidc::routes())
        .merge(ab::routes())
        .merge(audit::routes())
        .merge(devices::routes())
}
