//! Unknown user and wrong password cost the same Argon2id verification and answer the same
//! 401. An LDAP account never falls back to a local password.

use deskhaven_core::auth::limiter::{Attempt, ClientAddr, LockStatus, PendingSubject};
use deskhaven_core::login_events::{LoginMethod, LoginResult};
use deskhaven_core::sessions::{ClientDevice, SessionError, SessionToken};
use deskhaven_core::state::AppState;
use deskhaven_core::tfa::{ChallengeSecret, LoginBinding, PendingLogin, SecondFactor};
use deskhaven_core::users::{AuthError, Authenticated, CredentialStamp, User};

use super::event::{self, Attempted};
use super::failure::Failure;
use super::request::AccountLogin;
use crate::ClientApiError;

pub struct LoggedIn {
    pub user: User,
    pub token: SessionToken,
}

pub struct Challenged {
    pub user: User,
    pub secret: ChallengeSecret,
}

pub enum PasswordStep {
    LoggedIn(LoggedIn),
    Challenged(Challenged),
}

pub(super) enum Completion {
    Password,
    SecondFactor(PendingSubject),
}

pub async fn login(
    state: &AppState,
    client: ClientAddr,
    request: &AccountLogin,
) -> Result<PasswordStep, ClientApiError> {
    let (method, outcome) = authenticate(state, client, request).await;
    let recorded = match &outcome {
        Ok(PasswordStep::LoggedIn(done)) => Some((Some(done.user.id), LoginResult::Success)),
        Ok(PasswordStep::Challenged(c)) => {
            Some((Some(c.user.id), LoginResult::SecondFactorRequired))
        }
        Err(failure) => failure.event_result().map(|result| (None, result)),
    };
    if let Some((user_id, result)) = recorded {
        let attempted = Attempted {
            username: &request.username,
            rustdesk_id: &request.device.rustdesk_id,
            method,
        };
        event::record(state, client, &attempted, user_id, result).await;
    }
    outcome.map_err(Failure::into_error)
}

async fn authenticate(
    state: &AppState,
    client: ClientAddr,
    request: &AccountLogin,
) -> (LoginMethod, Result<PasswordStep, Failure>) {
    let attempt = match state
        .login_throttle
        .pre_check(client, &request.username)
        .await
    {
        Ok(attempt) => attempt,
        Err(err) => return (LoginMethod::Password, Err(err.into())),
    };
    let checked = state
        .ldap
        .password_login(&request.username, &request.password)
        .await;
    let step = password_step(state, client, request, attempt, checked.result).await;
    (checked.method, step)
}

async fn password_step(
    state: &AppState,
    client: ClientAddr,
    request: &AccountLogin,
    attempt: Attempt,
    verified: Result<Authenticated, AuthError>,
) -> Result<PasswordStep, Failure> {
    let Authenticated { user, stamp } = match verified {
        Ok(verified) => verified,
        Err(err) => return Err(fail(state, client, attempt, err.into()).await),
    };
    match state.tfa.requirement(&user) {
        SecondFactor::NotRequired => {
            let session = Session {
                device: &request.device,
                user,
                stamp,
            };
            issue_session(state, client, attempt, session, Completion::Password)
                .await
                .map(PasswordStep::LoggedIn)
        }
        SecondFactor::Required => challenge(state, request, attempt, user, &stamp).await,
        SecondFactor::SetupRequired => Err(counted(state, client, attempt, Failure::SetupRequired)),
    }
}

async fn challenge(
    state: &AppState,
    request: &AccountLogin,
    attempt: Attempt,
    user: User,
    stamp: &CredentialStamp,
) -> Result<PasswordStep, Failure> {
    let lockout = state.login_throttle.on_password_ok_pending_2fa(attempt);
    let pending = PendingLogin {
        user_id: user.id,
        stamp,
        binding: LoginBinding::Device(&request.device),
        lockout: &lockout,
    };
    let secret = state
        .tfa
        .issue_challenge(&pending)
        .await
        .map_err(|err| Failure::Internal(err.into()))?;
    Ok(PasswordStep::Challenged(Challenged { user, secret }))
}

async fn fail(state: &AppState, client: ClientAddr, attempt: Attempt, failure: Failure) -> Failure {
    let throttle = &state.login_throttle;
    match failure {
        Failure::Busy => {
            if let Err(err) = throttle.refund(attempt).await {
                tracing::warn!(error = %err, "cannot refund a login attempt the busy hasher never verified");
            }
            Failure::Busy
        }
        // The attempt is dropped, which counts it.
        internal @ Failure::Internal(_) => internal,
        verified => counted(state, client, attempt, verified),
    }
}

pub(super) fn counted(
    state: &AppState,
    client: ClientAddr,
    attempt: Attempt,
    failure: Failure,
) -> Failure {
    match state.login_throttle.on_failure(attempt) {
        LockStatus::Locked { retry_after } => {
            tracing::info!(ip = %client.addr, ?retry_after, "login failures started a lockout");
            Failure::LockStarted(Box::new(failure))
        }
        LockStatus::Allowed => failure,
    }
}

pub(super) struct Session<'a> {
    pub(super) device: &'a ClientDevice,
    pub(super) user: User,
    pub(super) stamp: CredentialStamp,
}

pub(super) async fn issue_session(
    state: &AppState,
    client: ClientAddr,
    attempt: Attempt,
    session: Session<'_>,
    completion: Completion,
) -> Result<LoggedIn, Failure> {
    let Session {
        device,
        user,
        stamp,
    } = session;
    let created = state
        .sessions
        .create_client(user.id, &stamp, device, Some(client.addr))
        .await;
    match created {
        Ok((_, token)) => {
            let throttle = &state.login_throttle;
            let released = match completion {
                Completion::Password => throttle.on_success(attempt).await,
                Completion::SecondFactor(pending) => throttle.complete(pending, attempt).await,
            };
            if let Err(err) = released {
                // The login itself succeeded; only the failure counters were not reset.
                tracing::warn!(user_id = user.id, error = %err, "cannot reset login failure counters");
            }
            Ok(LoggedIn { user, token })
        }
        // The password changed (or the account was disabled) while logging in.
        Err(SessionError::StaleCredential) => {
            Err(counted(state, client, attempt, Failure::InvalidCredentials))
        }
        // The attempt is dropped, which counts it.
        Err(err) => Err(Failure::Internal(err.into())),
    }
}
