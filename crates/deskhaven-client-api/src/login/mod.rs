//! Requiring `application/json` keeps browsers out: a cross-site form or `text/plain` fetch
//! needs no CORS preflight, so any web page could otherwise spend its visitors' login budget.
//! The official client's 2FA field submits only six digits, so recovery codes are for the web
//! console. The client replays a 5xx once and ignores `Retry-After`; a busy-hasher 503
//! refunds its attempt, so the replay is one more attempt.

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

/// The body is parsed before anything else limits the caller, so the 2 MiB stack limit would
/// let every request cost tens of MiB of JSON values.
pub const MAX_LOGIN_BODY: usize = 16 * 1024;

const ACCESS_TOKEN: &str = "access_token";
const EMAIL_CHECK: &str = "email_check";
const TFA_CHECK: &str = "tfa_check";

/// An untagged union: each serializes as its own object.
#[derive(Serialize)]
#[serde(untagged)]
pub enum LoginReply {
    Token(TokenReply),
    Challenge(ChallengeReply),
}

/// `tfa_type` / `secret` are left out: the Rust reader of the same AuthBody (OIDC) rejects
/// them as `null`, and Flutter treats absent as null.
#[derive(Serialize)]
pub struct TokenReply {
    access_token: String,
    #[serde(rename = "type")]
    kind: &'static str,
    user: UserPayload,
}

/// `user.name` is required: the code dialog sends it back as the second step's `username`.
/// Nothing else about the account is disclosed before the second factor.
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
    /// Also the OIDC poll's, read by the Rust client.
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
