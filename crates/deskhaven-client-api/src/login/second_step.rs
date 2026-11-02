//! Second step of a two-step login (`type: "email_code"` answering a `tfa_check`
//! challenge), in the order the login throttle requires:
//!
//! 1. `pre_check` with the request's username: the same username key as the first step
//!    (the challenge only verifies for the username it was issued to), so password and
//!    second-factor guesses share one budget. Locked or over the limit -> 429 before the
//!    challenge is even looked up.
//! 2. `TfaService::verify_challenge`: claims one attempt of the challenge, checks its
//!    username and device binding, spends the code (TOTP step or recovery code) and
//!    consumes the challenge.
//! 3. Passed -> the client session under the first step's credential stamp (a password
//!    change or disable in between is a counted 401), then `complete` with the challenge's
//!    lockout subject. Wrong code (401 `wrong-2fa-code`, the dialog stays open) or no usable
//!    challenge (401, log in again) -> `on_failure`.

use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::login_events::{LoginMethod, LoginResult};
use deskhaven_core::state::AppState;
use deskhaven_core::tfa::{SecondFactorMethod, SecondStep};

use super::account::{Completion, LoggedIn, Session, counted, issue_session};
use super::event::{self, Attempted};
use super::failure::Failure;
use super::request::SecondStepLogin;
use crate::ClientApiError;

/// The whole second step, with its login event.
pub async fn login(
    state: &AppState,
    client: ClientAddr,
    request: &SecondStepLogin,
) -> Result<LoggedIn, ClientApiError> {
    let outcome = verify(state, client, request).await;
    let recorded = match &outcome {
        Ok(done) => Some((Some(done.user.id), LoginResult::Success)),
        Err(failure) => failure.event_result().map(|result| (None, result)),
    };
    if let Some((user_id, result)) = recorded {
        let method = match SecondFactorMethod::of_code(&request.code) {
            SecondFactorMethod::Totp => LoginMethod::Totp,
            SecondFactorMethod::Recovery => LoginMethod::Recovery,
        };
        let attempted = Attempted {
            username: &request.username,
            rustdesk_id: &request.device.rustdesk_id,
            method,
        };
        event::record(state, client, &attempted, user_id, result).await;
    }
    outcome.map_err(Failure::into_error)
}

async fn verify(
    state: &AppState,
    client: ClientAddr,
    request: &SecondStepLogin,
) -> Result<LoggedIn, Failure> {
    let attempt = state
        .login_throttle
        .pre_check(client, &request.username)
        .await?;
    let step = SecondStep {
        secret: &request.secret,
        username: &request.username,
        device: &request.device,
        code: &request.code,
    };
    match state.tfa.verify_challenge(&step).await {
        Ok(passed) => {
            let session = Session {
                device: &request.device,
                user: passed.user,
                stamp: passed.stamp,
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
