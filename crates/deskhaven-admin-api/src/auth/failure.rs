//! Why a console sign-in step failed: its error reply and its login event.

use std::time::Duration;

use deskhaven_core::auth::limiter::{Attempt, ClientAddr, LockStatus, ThrottleError};
use deskhaven_core::login_events::LoginResult;
use deskhaven_core::state::AppState;
use deskhaven_core::tfa::ChallengeError;
use deskhaven_core::users::{AuthError, PasswordError};

use crate::{AdminApiError, BoxError, ErrorCode};

/// `Retry-After` of a 503 for a busy password hasher (its default acquire timeout).
pub(crate) const BUSY_RETRY_AFTER: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub(crate) enum Failure {
    Throttled(Duration),
    InvalidCredentials,
    Disabled,
    Unverified,
    /// Second step: wrong, replayed or used code.
    InvalidCode,
    /// Second step: no usable challenge.
    InvalidChallenge,
    /// A verified failure (one of the above) that started a lockout.
    LockStarted(Box<Failure>),
    /// Nothing verified: the password hasher is saturated or the directory is unavailable.
    Busy,
    Internal(BoxError),
}

impl Failure {
    /// The `login_events` result, or `None` for attempts that are not recorded (nothing
    /// was verified, or the server failed: logged instead).
    pub(crate) fn event_result(&self) -> Option<LoginResult> {
        match self {
            Self::InvalidCredentials => Some(LoginResult::InvalidCredentials),
            Self::Disabled => Some(LoginResult::Disabled),
            Self::Unverified => Some(LoginResult::Unverified),
            Self::InvalidCode => Some(LoginResult::InvalidCode),
            Self::InvalidChallenge => Some(LoginResult::InvalidChallenge),
            Self::LockStarted(_) => Some(LoginResult::Locked),
            Self::Throttled(_) | Self::Busy | Self::Internal(_) => None,
        }
    }

    pub(crate) fn into_error(self) -> AdminApiError {
        let error = |code, message| AdminApiError::new(code, message);
        match self {
            Self::Throttled(retry_after) => AdminApiError::rate_limited(retry_after),
            Self::InvalidCredentials => {
                error(ErrorCode::InvalidCredentials, "wrong username or password")
            }
            Self::Disabled => error(ErrorCode::AccountDisabled, "the account is disabled"),
            Self::Unverified => error(ErrorCode::AccountUnverified, "the account is not verified"),
            Self::InvalidCode => error(ErrorCode::InvalidCode, "wrong code"),
            Self::InvalidChallenge => error(
                ErrorCode::InvalidChallenge,
                "the verification expired, log in again",
            ),
            Self::LockStarted(failure) => failure.into_error(),
            Self::Busy => AdminApiError::unavailable(BUSY_RETRY_AFTER),
            Self::Internal(err) => AdminApiError::internal(err),
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
            // Nothing verified either way: refunded, 503.
            AuthError::Password(PasswordError::Busy) | AuthError::DirectoryUnavailable => {
                Self::Busy
            }
            other => Self::Internal(other.into()),
        }
    }
}

impl From<ChallengeError> for Failure {
    fn from(err: ChallengeError) -> Self {
        match err {
            ChallengeError::Invalid => Self::InvalidChallenge,
            ChallengeError::WrongCode => Self::InvalidCode,
            ChallengeError::Internal(err) => Self::Internal(err.into()),
        }
    }
}

/// A verified failure: the attempt stays counted; marks the one that started a lockout.
pub(crate) fn counted(
    state: &AppState,
    client: ClientAddr,
    attempt: Attempt,
    failure: Failure,
) -> Failure {
    match state.login_throttle.on_failure(attempt) {
        LockStatus::Locked { retry_after } => {
            tracing::info!(ip = %client.addr, ?retry_after, "console login failures started a lockout");
            Failure::LockStarted(Box::new(failure))
        }
        LockStatus::Allowed => failure,
    }
}

/// Reports a failed password verification to the throttle (busy hasher: refunded).
pub(crate) async fn failed(
    state: &AppState,
    client: ClientAddr,
    attempt: Attempt,
    failure: Failure,
) -> Failure {
    match failure {
        Failure::Busy => {
            if let Err(err) = state.login_throttle.refund(attempt).await {
                tracing::warn!(error = %err, "cannot refund a login attempt the busy hasher never verified");
            }
            Failure::Busy
        }
        // The attempt is dropped, which counts it.
        internal @ Failure::Internal(_) => internal,
        verified => counted(state, client, attempt, verified),
    }
}
