use std::time::Duration;

use deskhaven_core::auth::limiter::ThrottleError;
use deskhaven_core::login_events::LoginResult;
use deskhaven_core::tfa::ChallengeError;
use deskhaven_core::users::{AuthError, PasswordError};

use crate::{BoxError, ClientApiError};

/// A client translation key.
pub const WRONG_CREDENTIALS_MESSAGE: &str = "Wrong credentials";
pub const DISABLED_MESSAGE: &str = "Account disabled";
pub const UNVERIFIED_MESSAGE: &str = "Account not verified";
pub const TFA_SETUP_REQUIRED_MESSAGE: &str =
    "Two-factor authentication is required: set it up in the web console first";
/// A client translation key.
pub const WRONG_CODE_MESSAGE: &str = "wrong-2fa-code";
/// The code dialog cannot succeed: the user must log in again.
pub const INVALID_CHALLENGE_MESSAGE: &str = "Verification expired, please log in again";
/// The hasher's default acquire timeout.
pub const BUSY_RETRY_AFTER: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub(super) enum Failure {
    Throttled(Duration),
    InvalidCredentials,
    Disabled,
    Unverified,
    SetupRequired,
    InvalidCode,
    InvalidChallenge,
    LockStarted(Box<Failure>),
    Busy,
    /// The directory went silent after the password was sent: 503, yet the attempt counts.
    Interrupted,
    Internal(BoxError),
}

impl Failure {
    /// `None` for attempts that are not recorded: refusals are unbounded in number, so recording
    /// them would turn every refused request into a database write.
    pub(super) fn event_result(&self) -> Option<LoginResult> {
        match self {
            Self::InvalidCredentials => Some(LoginResult::InvalidCredentials),
            Self::Disabled => Some(LoginResult::Disabled),
            Self::Unverified => Some(LoginResult::Unverified),
            Self::SetupRequired => Some(LoginResult::SecondFactorSetupRequired),
            Self::InvalidCode => Some(LoginResult::InvalidCode),
            Self::InvalidChallenge => Some(LoginResult::InvalidChallenge),
            Self::LockStarted(_) => Some(LoginResult::Locked),
            Self::Throttled(_) | Self::Busy | Self::Interrupted | Self::Internal(_) => None,
        }
    }

    pub(super) fn into_error(self) -> ClientApiError {
        let unauthorized = |m: &'static str| ClientApiError::Unauthorized(m.into());
        let forbidden = |m: &'static str| ClientApiError::Forbidden(m.into());
        match self {
            Self::Throttled(retry_after) => ClientApiError::TooManyRequests { retry_after },
            Self::InvalidCredentials => unauthorized(WRONG_CREDENTIALS_MESSAGE),
            Self::Disabled => forbidden(DISABLED_MESSAGE),
            Self::Unverified => forbidden(UNVERIFIED_MESSAGE),
            Self::SetupRequired => forbidden(TFA_SETUP_REQUIRED_MESSAGE),
            Self::InvalidCode => unauthorized(WRONG_CODE_MESSAGE),
            Self::InvalidChallenge => unauthorized(INVALID_CHALLENGE_MESSAGE),
            Self::LockStarted(failure) => failure.into_error(),
            Self::Busy | Self::Interrupted => ClientApiError::Unavailable {
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
            // Nothing verified either way: refunded, 503.
            AuthError::Password(PasswordError::Busy) | AuthError::DirectoryUnavailable => {
                Self::Busy
            }
            AuthError::DirectoryInterrupted => Self::Interrupted,
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
