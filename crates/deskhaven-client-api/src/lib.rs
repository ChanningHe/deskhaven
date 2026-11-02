//! HTTP API consumed by the official RustDesk client (`/api/*`).
//!
//! Every endpoint is contract-tested by fixtures in `tests/contract`
//! (crate `deskhaven-contract`, which this crate must never depend on).

mod error;
mod login_options;
mod stack_error;

use axum::Router;
use axum::routing::get;

pub use error::{BoxError, ClientApiError};
pub use stack_error::render_stack_error;

/// Client API routes (no middleware; the server applies the stack).
pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new().route("/api/login-options", get(login_options::login_options))
}
