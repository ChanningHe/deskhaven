//! `POST /api/login` (03-client-api-contract §3.2). Read by Flutter
//! (`flutter/lib/models/user_model.dart:193-235`): any non-200 shows `translate(error)`;
//! a 200 must be an AuthBody, which the client stores when `type` is `access_token`.
//!
//! | case | status | body |
//! |---|---|---|
//! | right password | 200 | `{access_token, type: "access_token", user}` |
//! | wrong password / unknown account | 401 | `{"error": "Wrong credentials"}` |
//! | disabled / unverified (right password only) | 403 | `{"error": "Account disabled" / "Account not verified"}` |
//! | IP over limit / account locked | 429 + `Retry-After` | `{"error": "Please try later"}` |
//! | password hasher busy | 503 + `Retry-After` | `{"error": "Please try later"}` |
//! | malformed body, not `application/json`, unsupported `type` | 400 | `{"error": ...}` |
//! | body over [`MAX_LOGIN_BODY`] | 413 | `{"error": "payload too large"}` (stack renderer) |
//!
//! Nothing is counted or verified for a 400 or 413: those are refused before the throttle.
//! Requiring `application/json` (the client always sends it, `http_service.dart:18`) keeps
//! browsers out: a cross-site form or `text/plain` fetch needs no CORS preflight, so
//! without it any web page could make its visitors spend their address's login budget.
//!
//! Retries: the client replays a request once through the hbbs TCP proxy when the API
//! answers 5xx (`src/common.rs`, `with_tcp_proxy_fallback`) and ignores `Retry-After`. A
//! busy-hasher 503 refunds its attempt, so the replay is just one more attempt.

mod account;
mod event;
mod request;

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use deskhaven_core::state::AppState;
use serde::Serialize;

use self::account::LoggedIn;
use self::request::{LoginRequest, UNSUPPORTED_TYPE_MESSAGE, require_json};

/// Largest accepted `/api/login` body (bytes): the request is a few hundred bytes, and the
/// body is parsed before anything else limits the caller, so the 2 MiB stack limit would
/// let every request cost tens of MiB of JSON values.
pub const MAX_LOGIN_BODY: usize = 16 * 1024;
use crate::ClientApiError;
use crate::client::Client;
use crate::user_payload::UserPayload;

/// `type` of a completed login (`HttpType.kAuthResTypeToken`).
const ACCESS_TOKEN: &str = "access_token";

/// A completed login. `tfa_type` / `secret` are left out: the Rust reader of the same
/// AuthBody (OIDC) rejects them as `null`, and Flutter treats absent as null.
#[derive(Serialize)]
pub struct LoginReply {
    access_token: String,
    #[serde(rename = "type")]
    kind: &'static str,
    user: UserPayload,
}

impl From<LoggedIn> for LoginReply {
    fn from(done: LoggedIn) -> Self {
        Self {
            access_token: done.token.expose().to_owned(),
            kind: ACCESS_TOKEN,
            user: UserPayload::from(&done.user),
        }
    }
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    Client(client): Client,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<LoginReply>, ClientApiError> {
    require_json(&headers)?;
    match LoginRequest::parse(&body)? {
        LoginRequest::Account(request) => account::login(&state, client, &request)
            .await
            .map(|done| Json(done.into())),
        LoginRequest::Unsupported => {
            Err(ClientApiError::BadRequest(UNSUPPORTED_TYPE_MESSAGE.into()))
        }
    }
}
