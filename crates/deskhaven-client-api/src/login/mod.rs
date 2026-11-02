//! `POST /api/login` (03-client-api-contract §3.2). Read by Flutter
//! (`flutter/lib/models/user_model.dart:193-235`): any non-200 shows `translate(error)`;
//! a 200 must be an AuthBody, which the client stores when `type` is `access_token`.
//!
//! Password step (`type: "account"`):
//!
//! | case | status | body |
//! |---|---|---|
//! | right password, no second factor | 200 | `{access_token, type: "access_token", user}` |
//! | right password, TOTP enabled | 200 | `{type: "email_check", tfa_type: "tfa_check", secret, user: {name}}` |
//! | wrong password / unknown account (2FA or not) | 401 | `{"error": "Wrong credentials"}` |
//! | disabled / unverified (right password only) | 403 | `{"error": "Account disabled" / "Account not verified"}` |
//! | `auth.tfa.enforce` covers the user, not enrolled (right password only) | 403 | `{"error": TFA_SETUP_REQUIRED_MESSAGE}` |
//!
//! Second step (`type: "email_code"`, the code dialog of `login.dart:1003-1046`, which
//! stays open on an error and shows `translate(error)`):
//!
//! | case | status | body |
//! |---|---|---|
//! | right code (TOTP, or a recovery code) | 200 | `{access_token, type: "access_token", user}` |
//! | wrong / replayed / used code | 401 | `{"error": "wrong-2fa-code"}` |
//! | unknown, expired, used or exhausted challenge; other username or device | 401 | `{"error": INVALID_CHALLENGE_MESSAGE}` |
//!
//! Both steps:
//!
//! | case | status | body |
//! |---|---|---|
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
//! Recovery codes are accepted in the second step, but the official client's 2FA field
//! only submits exactly six digits (`Dialog2FaField`, `dialog.dart:578-640`): from the
//! client a user can only enter TOTP codes; recovery codes are for the web console.
//!
//! Retries: the client replays a request once through the hbbs TCP proxy when the API
//! answers 5xx (`src/common.rs`, `with_tcp_proxy_fallback`) and ignores `Retry-After`. A
//! busy-hasher 503 refunds its attempt, so the replay is just one more attempt.

mod account;
pub(crate) mod event;
pub(crate) mod failure;
pub(crate) mod request;
mod second_step;

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use deskhaven_core::sessions::SessionToken;
use deskhaven_core::state::AppState;
use deskhaven_core::users::User;
use serde::Serialize;

use self::account::{Challenged, LoggedIn, PasswordStep};
use self::request::{LoginRequest, UNSUPPORTED_TYPE_MESSAGE, require_json};
use crate::ClientApiError;
use crate::client::Client;
use crate::user_payload::UserPayload;

/// Largest accepted `/api/login` body (bytes): the request is a few hundred bytes, and the
/// body is parsed before anything else limits the caller, so the 2 MiB stack limit would
/// let every request cost tens of MiB of JSON values.
pub const MAX_LOGIN_BODY: usize = 16 * 1024;

/// `type` of a completed login (`HttpType.kAuthResTypeToken`).
const ACCESS_TOKEN: &str = "access_token";
/// `type` of a reply that wants a second step (`HttpType.kAuthResTypeEmailCheck`).
const EMAIL_CHECK: &str = "email_check";
/// `tfa_type` of a TOTP challenge (`HttpType.kAuthResTypeTfaCheck`).
const TFA_CHECK: &str = "tfa_check";

/// `/api/login` replies (an untagged union: each serializes as its own object).
#[derive(Serialize)]
#[serde(untagged)]
pub enum LoginReply {
    Token(TokenReply),
    Challenge(ChallengeReply),
}

/// A completed login. `tfa_type` / `secret` are left out: the Rust reader of the same
/// AuthBody (OIDC) rejects them as `null`, and Flutter treats absent as null.
#[derive(Serialize)]
pub struct TokenReply {
    access_token: String,
    #[serde(rename = "type")]
    kind: &'static str,
    user: UserPayload,
}

/// A second factor is due (`login.dart:790-812`). No `access_token`. `user.name` is
/// required: the code dialog sends it back as the second step's `username`
/// (`login.dart:1020`); nothing else about the account is disclosed before the second
/// factor.
#[derive(Serialize)]
pub struct ChallengeReply {
    #[serde(rename = "type")]
    kind: &'static str,
    tfa_type: &'static str,
    secret: String,
    user: ChallengeUser,
}

#[derive(Serialize)]
pub struct ChallengeUser {
    name: String,
}

impl TokenReply {
    /// The AuthBody of a completed login (also the OIDC poll's, read by the Rust client).
    pub(crate) fn new(user: &User, token: &SessionToken) -> Self {
        Self {
            access_token: token.expose().to_owned(),
            kind: ACCESS_TOKEN,
            user: UserPayload::from(user),
        }
    }
}

impl From<LoggedIn> for LoginReply {
    fn from(done: LoggedIn) -> Self {
        Self::Token(TokenReply::new(&done.user, &done.token))
    }
}

impl From<Challenged> for LoginReply {
    fn from(challenged: Challenged) -> Self {
        Self::Challenge(ChallengeReply {
            kind: EMAIL_CHECK,
            tfa_type: TFA_CHECK,
            secret: challenged.secret.expose().to_owned(),
            user: ChallengeUser {
                name: challenged.user.username,
            },
        })
    }
}

impl From<PasswordStep> for LoginReply {
    fn from(step: PasswordStep) -> Self {
        match step {
            PasswordStep::LoggedIn(done) => done.into(),
            PasswordStep::Challenged(challenged) => challenged.into(),
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
    let reply: LoginReply = match LoginRequest::parse(&body)? {
        LoginRequest::Account(request) => account::login(&state, client, &request).await?.into(),
        LoginRequest::SecondStep(request) => {
            second_step::login(&state, client, &request).await?.into()
        }
        LoginRequest::Unsupported => {
            return Err(ClientApiError::BadRequest(UNSUPPORTED_TYPE_MESSAGE.into()));
        }
    };
    Ok(Json(reply))
}
