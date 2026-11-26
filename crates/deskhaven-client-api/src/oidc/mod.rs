//! Nothing proves that `id` is the device's own, and ids are shared routinely for remote
//! support, so an id registered here with another uuid is refused; an id not registered yet
//! passes, and the confirmation page says the server cannot vouch for it. `auth-query` is
//! not rate limited: a login polls up to 180 times, each one indexed lookup of a hashed
//! 256-bit code.

mod auth;
mod callback;
mod page;
mod params;
mod query;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use std::sync::Arc;

use deskhaven_core::oidc::Failure;
use deskhaven_core::state::AppState;

use crate::login::MAX_LOGIN_BODY;
use crate::login::failure::{DISABLED_MESSAGE, UNVERIFIED_MESSAGE};

pub const NOT_CONFIGURED_MESSAGE: &str = "OIDC login is not configured on this server";
pub const UNKNOWN_PROVIDER_MESSAGE: &str = "Unknown OIDC provider";
pub const PROVIDER_UNAVAILABLE_MESSAGE: &str = "The OIDC provider is unavailable, please try later";
/// The client keeps polling on exactly this message.
pub const PENDING_MESSAGE: &str = "No authed oidc is found";
pub const INVALID_LOGIN_MESSAGE: &str = "OIDC login expired, please log in again";
pub const ID_TAKEN_MESSAGE: &str = "This RustDesk ID belongs to another device on this server";

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/api/oidc/auth",
            post(auth::auth).layer(DefaultBodyLimit::max(MAX_LOGIN_BODY)),
        )
        .route("/api/oidc/auth-query", get(query::auth_query))
        .route(
            "/api/oidc/callback",
            get(callback::confirm).post(callback::complete),
        )
}

pub(crate) const fn failure_message(failure: Failure) -> &'static str {
    match failure {
        Failure::Denied => "OIDC login was cancelled or denied",
        Failure::Unavailable => PROVIDER_UNAVAILABLE_MESSAGE,
        Failure::InvalidToken => "OIDC login failed: the provider's answer could not be verified",
        Failure::NoLinkedAccount => {
            "No account is linked to this OIDC login: sign in to the web console with your existing account and link it under My account, Linked sign-ins"
        }
        Failure::Disabled => DISABLED_MESSAGE,
        Failure::Unverified => UNVERIFIED_MESSAGE,
        Failure::ProvisioningFailed => "No account could be created for this OIDC login",
    }
}
