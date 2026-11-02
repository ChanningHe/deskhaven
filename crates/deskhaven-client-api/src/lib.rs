//! HTTP API consumed by the official RustDesk client (`/api/*`).
//!
//! Every endpoint is contract-tested by fixtures in `tests/contract`
//! (crate `deskhaven-contract`, which this crate must never depend on).

mod auth;
mod error;
mod login_options;
mod stack_error;

use std::sync::Arc;

use axum::Router;
use axum::routing::get;
use deskhaven_core::state::AppState;

pub use auth::{BearerUser, UNAUTHORIZED_MESSAGE};
pub use error::{BoxError, ClientApiError};
pub use stack_error::render_stack_error;

/// Client API routes, without middleware or state: the server applies the
/// stack and provides the state (`with_state`) when composing. Handlers that
/// need services take `State<Arc<AppState>>`.
pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/api/login-options", get(login_options::login_options))
}
