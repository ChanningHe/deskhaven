//! HTTP API consumed by the official RustDesk client (`/api/*`).
//!
//! Every endpoint is contract-tested by fixtures in `tests/contract`
//! (crate `deskhaven-contract`, which this crate must never depend on).

mod account;
mod auth;
mod client;
mod error;
mod login;
mod login_options;
mod stack_error;
mod user_payload;

use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};
use deskhaven_core::state::AppState;

pub use auth::{BearerUser, UNAUTHORIZED_MESSAGE};
pub use error::{BoxError, ClientApiError, TRY_LATER_MESSAGE};
pub use stack_error::render_stack_error;

/// Client API routes, without middleware or state: the server applies the
/// stack and provides the state (`with_state`) when composing. Handlers that
/// need services take `State<Arc<AppState>>`.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/login-options", get(login_options::login_options))
        .route("/api/login", post(login::login))
        .route("/api/currentUser", post(account::current_user))
        .route("/api/logout", post(account::logout))
}
