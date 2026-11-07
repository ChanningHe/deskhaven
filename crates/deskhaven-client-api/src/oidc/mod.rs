//! OIDC login of the RustDesk client (03 §3.5–3.6, 06 §3.4), over
//! `deskhaven_core::oidc`. The Rust client (`src/hbbs_http/account.rs:160-340`) posts
//! `/api/oidc/auth`, opens the returned `url` in the browser, and polls
//! `/api/oidc/auth-query` every second for up to 3 minutes. It reads bodies only
//! (`HbbHttpResponse::parse`): any `error` other than `"No authed oidc is found"` ends the
//! login with that message; a 5xx makes it retry once through the hbbs proxy.
//!
//! `POST /api/oidc/auth` (`{op, id, uuid, deviceInfo, apiDomain}`; `apiDomain` is ignored,
//! the callback URL is fixed by `public_url`):
//!
//! | case | status | body |
//! |---|---|---|
//! | flow started | 200 | `{code, url}` |
//! | malformed, not `application/json`, bad device | 400 | `{"error": "Invalid login request"}` |
//! | no `public_url` | 404 | `{"error": NOT_CONFIGURED_MESSAGE}` |
//! | unknown or disabled `op` | 404 | `{"error": UNKNOWN_PROVIDER_MESSAGE}` |
//! | IP over `auth.ip_rate_limit.per_minute` | 429 + `Retry-After` | `{"error": "Please try later"}` |
//! | IdP discovery failed | 502 | `{"error": PROVIDER_UNAVAILABLE_MESSAGE}` |
//!
//! Only the per-IP limiter applies (there is no username to count, see
//! `deskhaven_core::auth::limiter`); it bounds flow rows and outbound discovery requests.
//!
//! `GET /api/oidc/auth-query?code=&id=&uuid=` (not rate limited: a login polls up to 180
//! times; each poll is one indexed lookup of a hashed 256-bit code):
//!
//! | case | status | body |
//! |---|---|---|
//! | callback not completed yet | 404 | `{"error": "No authed oidc is found"}` |
//! | completed (once) | 200 | AuthBody `{access_token, type: "access_token", user}` |
//! | flow failed | 403 | `{"error": <the failure's message>}` |
//! | unknown, expired, already used code; another device; account changed since | 401 | `{"error": INVALID_LOGIN_MESSAGE}` |
//! | missing parameter | 400 | `{"error": "Invalid login request"}` |
//!
//! `GET <public_url>/api/oidc/callback` is the browser's redirect from the IdP: it shows which
//! device would be signed in and a button; the button's `POST` completes the flow (see
//! `callback`). Both always answer 200 with a small HTML page.

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
/// The client keeps polling on exactly this message (`account.rs:323`).
pub const PENDING_MESSAGE: &str = "No authed oidc is found";
pub const INVALID_LOGIN_MESSAGE: &str = "OIDC login expired, please log in again";

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

/// What the user is told about a failed flow (client dialog and callback page).
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
