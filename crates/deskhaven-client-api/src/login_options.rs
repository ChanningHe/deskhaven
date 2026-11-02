//! `GET /api/login-options` (03-client-api-contract §3.1): third-party login
//! buttons as `"oidc/<name>"` strings. The Rust client also calls it as a TLS
//! probe, so it takes no auth, touches nothing slow and always answers 200.

use axum::Json;

/// No OIDC providers until M2-10, hence always an empty list.
pub async fn login_options() -> Json<Vec<String>> {
    Json(Vec::new())
}
