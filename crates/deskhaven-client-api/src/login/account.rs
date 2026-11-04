//! Password login (`type: "account"`), in the order the login throttle requires
//! (`deskhaven_core::auth::limiter`, "Handler protocol"):
//!
//! 1. `pre_check`: per-IP token, then one attempt reserved (counted) for the username from
//!    this source. Locked or over the limit -> 429; a database error fails closed (500, no
//!    verification).
//! 2. `LdapService::authenticate_password`: the local password, or the directory for LDAP
//!    accounts (and unknown names while a directory is enabled), never both. One Argon2id
//!    verification either way, real or dummy (unknown accounts), so unknown user and wrong
//!    password cost the same and answer the same 401, whether or not the account has 2FA.
//! 3. Wrong password, unknown, disabled or unverified account -> `on_failure` (the attempt
//!    stays counted). Hasher busy or directory unavailable -> `refund` (never counted) and
//!    503: nothing was verified, and an LDAP account never falls back to a local password.
//! 4. Right password, by the user's second factor (`TfaService::requirement`):
//!    - none: the client session (bound to the request's device, re-checking the
//!      credential stamp; a credential changed meanwhile is a counted 401), then
//!      `on_success`;
//!    - TOTP: `on_password_ok_pending_2fa` (the attempt stays counted, no `on_success`),
//!      a challenge holding the returned subject, and the challenge reply;
//!    - enforced but not enrolled: counted, 403 (enrolment is in the web console).

use deskhaven_core::auth::limiter::{Attempt, ClientAddr, LockStatus, PendingSubject};
use deskhaven_core::login_events::{LoginMethod, LoginResult};
use deskhaven_core::sessions::{ClientDevice, SessionError, SessionToken};
use deskhaven_core::state::AppState;
use deskhaven_core::tfa::{ChallengeSecret, LoginBinding, PendingLogin, SecondFactor};
use deskhaven_core::users::{Authenticated, CredentialStamp, User};

use super::event::{self, Attempted};
use super::failure::Failure;
use super::request::AccountLogin;
use crate::ClientApiError;

/// A logged-in user and the new client session's token.
pub struct LoggedIn {
    pub user: User,
    pub token: SessionToken,
}

/// The password was right and a second factor is due.
pub struct Challenged {
    pub user: User,
    pub secret: ChallengeSecret,
}

/// Outcome of a right password.
pub enum PasswordStep {
    LoggedIn(LoggedIn),
    Challenged(Challenged),
}

/// What completes the login throttle once the session exists.
pub(super) enum Completion {
    /// One-step login: `on_success`.
    Password,
    /// Second factor passed: `complete` with the first step's subject.
    SecondFactor(PendingSubject),
}

/// The whole password step, with its login event.
pub async fn login(
    state: &AppState,
    client: ClientAddr,
    request: &AccountLogin,
) -> Result<PasswordStep, ClientApiError> {
    let outcome = authenticate(state, client, request).await;
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
            method: LoginMethod::Password,
        };
        event::record(state, client, &attempted, user_id, result).await;
    }
    outcome.map_err(Failure::into_error)
}

async fn authenticate(
    state: &AppState,
    client: ClientAddr,
    request: &AccountLogin,
) -> Result<PasswordStep, Failure> {
    let throttle = &state.login_throttle;
    let attempt = throttle.pre_check(client, &request.username).await?;
    let verified = state
        .ldap
        .authenticate_password(&request.username, &request.password)
        .await;
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

/// Keeps the attempt counted and stores the challenge carrying its subject.
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

/// Reports a failed verification to the throttle.
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

/// A verified failure: the attempt stays counted; marks the one that started a lockout.
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

/// Who gets a session, on which device, under which credential stamp.
pub(super) struct Session<'a> {
    pub(super) device: &'a ClientDevice,
    pub(super) user: User,
    pub(super) stamp: CredentialStamp,
}

/// The client session, then the throttle completion.
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
