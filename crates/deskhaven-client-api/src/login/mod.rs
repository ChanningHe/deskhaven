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
//! | malformed body / unsupported `type` | 400 | `{"error": ...}` |

mod account;
mod event;
mod request;

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use deskhaven_core::state::AppState;
use serde::Serialize;

use self::account::LoggedIn;
use self::request::{LoginRequest, UNSUPPORTED_TYPE_MESSAGE};
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
    body: Bytes,
) -> Result<Json<LoginReply>, ClientApiError> {
    match LoginRequest::parse(&body)? {
        LoginRequest::Account(request) => account::login(&state, client, &request)
            .await
            .map(|done| Json(done.into())),
        LoginRequest::Unsupported => {
            Err(ClientApiError::BadRequest(UNSUPPORTED_TYPE_MESSAGE.into()))
        }
    }
}
