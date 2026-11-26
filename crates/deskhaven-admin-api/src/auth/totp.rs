//! `pre_check` uses the first step's username, so password and code guesses share one budget.

use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::login_events::{LoginMethod, LoginResult, MAX_USERNAME_ATTEMPTED_CHARS};
use deskhaven_core::state::AppState;
use deskhaven_core::tfa::{LoginBinding, SecondFactorMethod, SecondStep};
use garde::Validate;
use serde::Deserialize;
use utoipa::ToSchema;

use super::event;
use super::failure::{Failure, counted};
use super::password::{Completion, NewSession, SignedIn, issue_session};
use super::{LoginResponse, revoke_presented, signed_in, user_agent};
use crate::session::Client;
use crate::transport::SecureTransport;
use crate::{AdminApiError, ValidJson};

const MAX_SECRET_INPUT: usize = 64;

/// The challenge of the password step and a code. Never logged.
#[derive(Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TotpLoginRequest {
    /// The username the password step was for.
    #[garde(length(chars, min = 1, max = MAX_USERNAME_ATTEMPTED_CHARS))]
    pub username: String,
    #[garde(length(chars, min = 1, max = MAX_SECRET_INPUT))]
    pub challenge: String,
    /// Six-digit TOTP code, or a recovery code (`xxxxx-xxxxx`).
    #[garde(length(chars, min = 1, max = MAX_SECRET_INPUT))]
    pub code: String,
}

/// Completes a password login with a TOTP or recovery code.
#[utoipa::path(
    post,
    path = "/auth/login/totp",
    operation_id = "login_totp",
    tag = "auth",
    security(()),
    request_body = TotpLoginRequest,
    responses(
        (status = 200, description = "Signed in (cookies set)", body = LoginResponse),
        (status = 401, description = "The challenge is gone: log in again (`invalid_challenge`)", body = crate::ErrorEnvelope),
        (status = 403, description = "Plain-HTTP console (`insecure_transport`), or `csrf_failed`", body = crate::ErrorEnvelope),
        (status = 422, description = "Wrong code (`invalid_code`), or malformed input (`validation_failed`)", body = crate::ErrorEnvelope),
        (status = 429, description = "Too many attempts (`rate_limited`)", body = crate::ErrorEnvelope),
    )
)]
pub async fn login(
    State(state): State<Arc<AppState>>,
    _secure: SecureTransport,
    Client(client): Client,
    headers: HeaderMap,
    ValidJson(request): ValidJson<TotpLoginRequest>,
) -> Result<Response, AdminApiError> {
    let outcome = verify(&state, client, &request, user_agent(&headers)).await;
    let recorded = match &outcome {
        Ok(done) => Some((Some(done.user.id), LoginResult::Success)),
        Err(failure) => failure.event_result().map(|result| (None, result)),
    };
    if let Some((user_id, result)) = recorded {
        let method = match SecondFactorMethod::of_code(&request.code) {
            SecondFactorMethod::Totp => LoginMethod::Totp,
            SecondFactorMethod::Recovery => LoginMethod::Recovery,
        };
        event::record(&state, client, &request.username, method, user_id, result).await;
    }
    let done = outcome.map_err(Failure::into_error)?;
    revoke_presented(&state, &headers).await;
    Ok(signed_in(&state, done.user, &done.session, &done.token))
}

async fn verify(
    state: &AppState,
    client: ClientAddr,
    request: &TotpLoginRequest,
    user_agent: Option<&str>,
) -> Result<SignedIn, Failure> {
    let attempt = state
        .login_throttle
        .pre_check(client, &request.username)
        .await?;
    let step = SecondStep {
        secret: &request.challenge,
        username: &request.username,
        binding: LoginBinding::Console,
        code: &request.code,
    };
    match state.tfa.verify_challenge(&step).await {
        Ok(passed) => {
            let session = NewSession {
                user: passed.user,
                stamp: passed.stamp,
                user_agent,
            };
            let completion = Completion::SecondFactor(passed.lockout);
            issue_session(state, client, attempt, session, completion).await
        }
        Err(err) => match Failure::from(err) {
            // The attempt is dropped, which counts it.
            internal @ Failure::Internal(_) => Err(internal),
            verified => Err(counted(state, client, attempt, verified)),
        },
    }
}
