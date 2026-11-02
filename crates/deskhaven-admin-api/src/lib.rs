//! Admin HTTP API (`/admin/api/v1`) backing the web console.
//!
//! - Routes: added only through [`AdminRoutes`] from `utoipa_axum::routes!`; each
//!   operation declares a [`Permission`] in its `#[utoipa::path]` (see [`permission`]).
//! - Gate (per route): same-origin and CSRF header on writes, console session cookie,
//!   role, console audit fallback (see [`session`], [`audit`]; writes record in their own
//!   transaction via [`AuditCtx`]).
//! - Wire conventions (08 §6): errors [`AdminApiError`], input [`ValidJson`] /
//!   [`ValidQuery`], lists [`PageQuery`] / [`Paginated`].
//! - OpenAPI: [`openapi_json`], committed at [`SPEC_PATH`] by `cargo xtask gen-openapi`.

pub mod audit;
mod error;
mod extract;
mod gate;
mod me;
mod openapi;
mod origin;
mod pagination;
pub mod permission;
mod routes;
pub mod session;
mod stack_error;

use std::sync::Arc;

use axum::Router;
use deskhaven_core::state::AppState;
use utoipa_axum::routes;

pub use audit::{Audit, AuditCtx, Audited};
pub use error::{AdminApiError, BoxError, ErrorBody, ErrorCode, ErrorEnvelope};
pub use extract::{MAX_REASON_CHARS, ValidJson, ValidQuery};
pub use me::{Me, RoleView};
pub use openapi::{OpenApiError, SPEC_PATH, openapi_json};
pub use origin::TrustedOrigins;
pub use pagination::{PageQuery, Paginated};
pub use permission::Permission;
pub use routes::{ADMIN_API_V1, AdminRoutes, RouteError};
pub use session::{
    CSRF_COOKIE, CSRF_HEADER, ConsoleUser, SESSION_COOKIE, clear_session_cookies, csrf_token,
    session_cookies,
};
pub use stack_error::render_stack_error;

/// Every production admin API route. Fails when an operation declares no valid
/// permission, so the server cannot start with an unguarded route.
pub fn api_routes() -> Result<AdminRoutes, RouteError> {
    AdminRoutes::new().routes(routes!(me::get_me))
}

/// The production routes behind their gates, for the admin listener.
pub fn router(state: Arc<AppState>) -> Result<Router, RouteError> {
    Ok(api_routes()?.into_router(state))
}
