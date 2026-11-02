//! Password login (`type: "account"`), in the order the login throttle requires
//! (`deskhaven_core::auth::limiter`, "Handler protocol"):
//!
//! 1. `pre_check`: per-IP token, then one attempt reserved (counted) for the username from
//!    this source. Locked or over the limit -> 429; a database error fails closed (500, no
//!    verification).
//! 2. `authenticate_local`: one Argon2id verification, real or dummy (unknown accounts), so
//!    unknown user and wrong password cost the same and answer the same 401.
//! 3. Wrong password, unknown, disabled or unverified account -> `on_failure` (the attempt
//!    stays counted). Hasher busy -> `refund` (never counted) and 503.
//! 4. Right password -> the client session (bound to the request's device, re-checking the
//!    credential stamp; a credential changed meanwhile is a counted 401), then `on_success`.

use std::time::Duration;

use deskhaven_core::auth::limiter::{Attempt, ClientAddr, LockStatus, ThrottleError};
use deskhaven_core::login_events::LoginResult;
use deskhaven_core::sessions::{SessionError, SessionToken};
use deskhaven_core::state::AppState;
use deskhaven_core::users::{AuthError, Authenticated, PasswordError, User};

use super::event;
use super::request::AccountLogin;
use crate::{BoxError, ClientApiError};

/// Client translation key (`src/lang/en.rs:57`: "Wrong username or password").
pub const WRONG_CREDENTIALS_MESSAGE: &str = "Wrong credentials";
pub const DISABLED_MESSAGE: &str = "Account disabled";
pub const UNVERIFIED_MESSAGE: &str = "Account not verified";
/// `Retry-After` of a 503 for a busy password hasher (its default acquire timeout).
pub const BUSY_RETRY_AFTER: Duration = Duration::from_secs(5);

/// Why a password login failed.
#[derive(Debug)]
enum Failure {
    Throttled(Duration),
    InvalidCredentials,
    Disabled,
    Unverified,
    /// A verified failure (one of the three above) that started a lockout.
    LockStarted(Box<Failure>),
    Busy,
    Internal(BoxError),
}

impl Failure {
    /// The `login_events` result, or `None` for attempts that are not recorded: nothing
    /// was verified (throttled, busy: unbounded in number, so recording them would turn
    /// every refused request into a database write) or the server failed (logged).
    fn event_result(&self) -> Option<LoginResult> {
        match self {
            Self::InvalidCredentials => Some(LoginResult::InvalidCredentials),
            Self::Disabled => Some(LoginResult::Disabled),
            Self::Unverified => Some(LoginResult::Unverified),
            Self::LockStarted(_) => Some(LoginResult::Locked),
            Self::Throttled(_) | Self::Busy | Self::Internal(_) => None,
        }
    }

    fn into_error(self) -> ClientApiError {
        match self {
            Self::Throttled(retry_after) => ClientApiError::TooManyRequests { retry_after },
            Self::InvalidCredentials => {
                ClientApiError::Unauthorized(WRONG_CREDENTIALS_MESSAGE.into())
            }
            Self::Disabled => ClientApiError::Forbidden(DISABLED_MESSAGE.into()),
            Self::Unverified => ClientApiError::Forbidden(UNVERIFIED_MESSAGE.into()),
            Self::LockStarted(failure) => failure.into_error(),
            Self::Busy => ClientApiError::Unavailable {
                retry_after: BUSY_RETRY_AFTER,
            },
            Self::Internal(err) => ClientApiError::Internal(err),
        }
    }
}

impl From<ThrottleError> for Failure {
    fn from(err: ThrottleError) -> Self {
        match err {
            ThrottleError::Throttled { retry_after } => Self::Throttled(retry_after),
            ThrottleError::Db(db) => Self::Internal(db.into()),
        }
    }
}

impl From<AuthError> for Failure {
    fn from(err: AuthError) -> Self {
        match err {
            AuthError::InvalidCredentials => Self::InvalidCredentials,
            AuthError::Disabled => Self::Disabled,
            AuthError::Unverified => Self::Unverified,
            AuthError::Password(PasswordError::Busy) => Self::Busy,
            other => Self::Internal(other.into()),
        }
    }
}

/// A logged-in user and the new client session's token.
pub struct LoggedIn {
    pub user: User,
    pub token: SessionToken,
}

/// The whole password login, with its login event.
pub async fn login(
    state: &AppState,
    client: ClientAddr,
    request: &AccountLogin,
) -> Result<LoggedIn, ClientApiError> {
    let outcome = authenticate(state, client, request).await;
    let recorded = match &outcome {
        Ok(done) => Some((Some(done.user.id), LoginResult::Success)),
        Err(failure) => failure.event_result().map(|result| (None, result)),
    };
    if let Some((user_id, result)) = recorded {
        event::record(state, client, request, user_id, result).await;
    }
    outcome.map_err(Failure::into_error)
}

async fn authenticate(
    state: &AppState,
    client: ClientAddr,
    request: &AccountLogin,
) -> Result<LoggedIn, Failure> {
    let throttle = &state.login_throttle;
    let attempt = throttle.pre_check(client, &request.username).await?;
    let verified = state
        .users
        .authenticate_local(&request.username, &request.password)
        .await;
    match verified {
        // The password is right. M2-08 hooks in here: when the user needs a second
        // factor, call `throttle.on_password_ok_pending_2fa(attempt)` instead of creating
        // the session (no `on_success`), store the returned subject with the challenge and
        // answer `{type: "email_check", tfa_type: "tfa_check", secret}`.
        Ok(verified) => issue_session(state, client, request, attempt, verified).await,
        Err(err) => Err(fail(state, client, attempt, err.into()).await),
    }
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
fn counted(state: &AppState, client: ClientAddr, attempt: Attempt, failure: Failure) -> Failure {
    match state.login_throttle.on_failure(attempt) {
        LockStatus::Locked { retry_after } => {
            tracing::info!(ip = %client.addr, ?retry_after, "login failures started a lockout");
            Failure::LockStarted(Box::new(failure))
        }
        LockStatus::Allowed => failure,
    }
}

async fn issue_session(
    state: &AppState,
    client: ClientAddr,
    request: &AccountLogin,
    attempt: Attempt,
    verified: Authenticated,
) -> Result<LoggedIn, Failure> {
    let Authenticated { user, stamp } = verified;
    let created = state
        .sessions
        .create_client(user.id, &stamp, &request.device, Some(client.addr))
        .await;
    match created {
        Ok((_, token)) => {
            if let Err(err) = state.login_throttle.on_success(attempt).await {
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
