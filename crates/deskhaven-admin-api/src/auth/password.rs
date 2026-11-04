//! `POST /auth/login`: the password step, in the login throttle's order:
//!
//! 1. `pre_check` (per-IP token, then one attempt reserved for the username from this
//!    source): locked or over the limit -> 429, nothing verified.
//! 2. `LdapService::authenticate_password`: the local password, or the directory for LDAP
//!    accounts (never both); one Argon2id verification either way, real or dummy, so
//!    unknown account and wrong password cost the same and answer the same 401
//!    `invalid_credentials`, whether or not the account has 2FA. Disabled / unverified
//!    (right password only): 403. Busy hasher or unavailable directory: refunded, 503.
//! 3. Right password, by the second factor (`TfaService::requirement`):
//!    - TOTP enabled: `on_password_ok_pending_2fa` (the attempt stays counted), a console
//!      challenge holding that subject, `totp_required`; **no session**;
//!    - otherwise (none, or enforced but not enrolled): the console session under the
//!      login's credential stamp, then `on_success`. An unenrolled account under
//!      `auth.tfa.enforce` gets a session restricted to the enrolment (`account_setup`),
//!      unlike the client, which has nowhere to enrol.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use deskhaven_core::auth::limiter::{Attempt, ClientAddr, PendingSubject};
use deskhaven_core::login_events::{LoginMethod, LoginResult, MAX_USERNAME_ATTEMPTED_CHARS};
use deskhaven_core::sessions::{Session, SessionError, SessionToken};
use deskhaven_core::state::AppState;
use deskhaven_core::tfa::{ChallengeSecret, LoginBinding, PendingLogin, SecondFactor};
use deskhaven_core::users::{Authenticated, CredentialStamp, Password, User};
use garde::Validate;
use serde::Deserialize;
use utoipa::ToSchema;

use super::event;
use super::failure::{Failure, counted, failed};
use super::{LoginResponse, revoke_presented, signed_in, user_agent};
use crate::session::Client;
use crate::transport::SecureTransport;
use crate::{AdminApiError, ValidJson};

/// Longest accepted password input (characters). Longer ones would fail anyway (the
/// policy maximum is 256); the bound keeps request bodies small.
pub(crate) const MAX_PASSWORD_INPUT: usize = 1024;

/// Username and password. Never logged.
#[derive(Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    #[garde(length(chars, min = 1, max = MAX_USERNAME_ATTEMPTED_CHARS))]
    pub username: String,
    #[garde(length(chars, min = 1, max = MAX_PASSWORD_INPUT))]
    #[schema(format = Password)]
    pub password: String,
}

/// A signed-in user and the new session.
pub(super) struct SignedIn {
    pub(super) user: User,
    pub(super) session: Session,
    pub(super) token: SessionToken,
}

/// Outcome of a right password.
enum Step {
    SignedIn(SignedIn),
    Challenged { user: User, secret: ChallengeSecret },
}

/// What completes the login throttle once the session exists.
pub(super) enum Completion {
    /// One-step login: `on_success`.
    Password,
    /// Second factor passed: `complete` with the first step's subject.
    SecondFactor(PendingSubject),
}

/// Signs in with a password (see the module docs).
#[utoipa::path(
    post,
    path = "/auth/login",
    operation_id = "login",
    tag = "auth",
    security(()),
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Signed in (cookies set), or a TOTP challenge", body = LoginResponse),
        (status = 401, description = "Wrong username or password (`invalid_credentials`)", body = crate::ErrorEnvelope),
        (status = 403, description = "Disabled or unverified account (`account_disabled`, `account_unverified`), plain-HTTP console (`insecure_transport`), or `csrf_failed`", body = crate::ErrorEnvelope),
        (status = 422, description = "Malformed input (`validation_failed`)", body = crate::ErrorEnvelope),
        (status = 429, description = "Too many attempts (`rate_limited`)", body = crate::ErrorEnvelope),
        (status = 503, description = "Password hashing saturated (`unavailable`)", body = crate::ErrorEnvelope),
    )
)]
pub async fn login(
    State(state): State<Arc<AppState>>,
    _secure: SecureTransport,
    Client(client): Client,
    headers: HeaderMap,
    ValidJson(request): ValidJson<LoginRequest>,
) -> Result<Response, AdminApiError> {
    let outcome = authenticate(&state, client, &request, user_agent(&headers)).await;
    let recorded = match &outcome {
        Ok(Step::SignedIn(done)) => Some((Some(done.user.id), LoginResult::Success)),
        Ok(Step::Challenged { user, .. }) => {
            Some((Some(user.id), LoginResult::SecondFactorRequired))
        }
        Err(failure) => failure.event_result().map(|result| (None, result)),
    };
    if let Some((user_id, result)) = recorded {
        let method = LoginMethod::Password;
        event::record(&state, client, &request.username, method, user_id, result).await;
    }
    match outcome.map_err(Failure::into_error)? {
        Step::SignedIn(done) => {
            revoke_presented(&state, &headers).await;
            Ok(signed_in(&state, done.user, &done.session, &done.token))
        }
        Step::Challenged { secret, .. } => {
            let body = LoginResponse::TotpRequired {
                challenge: secret.expose().to_owned(),
            };
            Ok(Json(body).into_response())
        }
    }
}

async fn authenticate(
    state: &AppState,
    client: ClientAddr,
    request: &LoginRequest,
    user_agent: Option<&str>,
) -> Result<Step, Failure> {
    let throttle = &state.login_throttle;
    let attempt = throttle.pre_check(client, &request.username).await?;
    let password = Password::new(request.password.as_str());
    let verified = state
        .ldap
        .authenticate_password(&request.username, &password)
        .await;
    let Authenticated { user, stamp } = match verified {
        Ok(verified) => verified,
        Err(err) => return Err(failed(state, client, attempt, err.into()).await),
    };
    match state.tfa.requirement(&user) {
        SecondFactor::Required => challenge(state, attempt, user, &stamp).await,
        SecondFactor::NotRequired | SecondFactor::SetupRequired => {
            let session = NewSession {
                user,
                stamp,
                user_agent,
            };
            issue_session(state, client, attempt, session, Completion::Password)
                .await
                .map(Step::SignedIn)
        }
    }
}

/// Keeps the attempt counted and stores the console challenge carrying its subject.
async fn challenge(
    state: &AppState,
    attempt: Attempt,
    user: User,
    stamp: &CredentialStamp,
) -> Result<Step, Failure> {
    let lockout = state.login_throttle.on_password_ok_pending_2fa(attempt);
    let pending = PendingLogin {
        user_id: user.id,
        stamp,
        binding: LoginBinding::Console,
        lockout: &lockout,
    };
    let secret = state
        .tfa
        .issue_challenge(&pending)
        .await
        .map_err(|err| Failure::Internal(err.into()))?;
    Ok(Step::Challenged { user, secret })
}

/// Who gets a console session, under which credential stamp.
pub(super) struct NewSession<'a> {
    pub(super) user: User,
    pub(super) stamp: CredentialStamp,
    pub(super) user_agent: Option<&'a str>,
}

/// The console session, then the throttle completion.
pub(super) async fn issue_session(
    state: &AppState,
    client: ClientAddr,
    attempt: Attempt,
    new: NewSession<'_>,
    completion: Completion,
) -> Result<SignedIn, Failure> {
    let NewSession {
        user,
        stamp,
        user_agent,
    } = new;
    let created = state
        .sessions
        .create_console(user.id, &stamp, Some(client.addr), user_agent)
        .await;
    match created {
        Ok((session, token)) => {
            let throttle = &state.login_throttle;
            let released = match completion {
                Completion::Password => throttle.on_success(attempt).await,
                Completion::SecondFactor(pending) => throttle.complete(pending, attempt).await,
            };
            if let Err(err) = released {
                // The login itself succeeded; only the failure counters were not reset.
                tracing::warn!(user_id = user.id, error = %err, "cannot reset login failure counters");
            }
            Ok(SignedIn {
                user,
                session,
                token,
            })
        }
        // The password changed (or the account was disabled) while logging in.
        Err(SessionError::StaleCredential) => {
            Err(counted(state, client, attempt, Failure::InvalidCredentials))
        }
        // The attempt is dropped, which counts it.
        Err(err) => Err(Failure::Internal(err.into())),
    }
}
