//! `pre_check` uses the first step's username, so password and second-factor guesses share
//! one budget.

use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::login_events::{LoginMethod, LoginResult};
use deskhaven_core::state::AppState;
use deskhaven_core::tfa::{LoginBinding, SecondFactorMethod, SecondStep};

use super::account::{Completion, LoggedIn, Session, counted, issue_session};
use super::event::{self, Attempted};
use super::failure::Failure;
use super::request::SecondStepLogin;
use crate::ClientApiError;

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
        binding: LoginBinding::Device(&request.device),
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
