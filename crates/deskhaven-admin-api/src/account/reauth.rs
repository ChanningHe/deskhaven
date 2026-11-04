//! Re-authentication with the current password for self-service changes (password change,
//! TOTP enrolment): a stolen session alone must not be able to take the account over.
//!
//! The check runs under the login throttle, like a login with the user's username:
//! `pre_check` (locked or over the limit: 429), then the verification; a wrong password is
//! a counted failure (a lockout started here is just as final) answered with **422**
//! `wrong_password`, never 401 (the console treats 401 as signed out). Right password:
//! `on_success`. Any other outcome (busy hasher, policy violation of a new password, a
//! server error) refunds the attempt.

use std::future::Future;

use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::state::AppState;
use deskhaven_core::users::{PasswordError, UserError};
use serde_json::json;

use crate::auth::{BUSY_RETRY_AFTER, throttle_error};
use crate::{AdminApiError, ErrorCode};

/// Longest accepted password input (characters; the policy maximum is 256).
pub(super) const MAX_PASSWORD_INPUT: usize = 1024;
/// Longest accepted code input (characters): a TOTP code or a recovery code.
pub(super) const MAX_CODE_INPUT: usize = 16;

/// Runs `verify` (which checks the current password of `username`'s account first) under
/// the login throttle; see the module docs.
pub(super) async fn with_current_password<T>(
    state: &AppState,
    client: ClientAddr,
    username: &str,
    verify: impl Future<Output = Result<T, UserError>>,
) -> Result<T, AdminApiError> {
    let checked = async { verify.await.map_err(password_failure) };
    throttled(state, client, username, checked).await
}

/// A failed re-authentication step: whether it counts against the login throttle.
pub(super) enum Failed {
    /// A wrong secret (password or code): a counted failure.
    Counted(AdminApiError),
    /// Anything else (busy hasher, policy violation, server error): the attempt is refunded.
    Refunded(AdminApiError),
}

/// The failure of a password check.
pub(super) fn password_failure(err: UserError) -> Failed {
    match err {
        UserError::WrongPassword => Failed::Counted(AdminApiError::new(
            ErrorCode::WrongPassword,
            "the current password is wrong",
        )),
        other => Failed::Refunded(user_error(other)),
    }
}

/// Runs `verify`, one or more re-authentication steps, as a single attempt of the login
/// throttle: `pre_check`, then `on_success`, `on_failure` or `refund` by its outcome.
pub(super) async fn throttled<T>(
    state: &AppState,
    client: ClientAddr,
    username: &str,
    verify: impl Future<Output = Result<T, Failed>>,
) -> Result<T, AdminApiError> {
    let throttle = &state.login_throttle;
    let attempt = throttle
        .pre_check(client, username)
        .await
        .map_err(throttle_error)?;
    match verify.await {
        Ok(value) => {
            if let Err(err) = throttle.on_success(attempt).await {
                tracing::warn!(error = %err, "cannot reset login failure counters");
            }
            Ok(value)
        }
        Err(Failed::Counted(err)) => {
            let _ = throttle.on_failure(attempt);
            Err(err)
        }
        Err(Failed::Refunded(err)) => {
            if let Err(refund) = throttle.refund(attempt).await {
                tracing::warn!(error = %refund, "cannot refund a re-authentication attempt");
            }
            Err(err)
        }
    }
}

/// 409 `not_local_account`: the account signs in elsewhere (OIDC), it has no password and
/// no local second factor.
pub(super) fn not_local() -> AdminApiError {
    AdminApiError::new(
        ErrorCode::NotLocalAccount,
        "the account has no local password",
    )
}

pub(super) fn user_error(err: UserError) -> AdminApiError {
    match err {
        UserError::NotFound => AdminApiError::unauthorized(),
        UserError::NotLocal => not_local(),
        UserError::PolicyViolation { reason } => {
            AdminApiError::validation_failed("the new password breaks the password policy")
                .with_details(json!({ "fields": { "new_password": [reason.to_string()] } }))
        }
        UserError::Password(PasswordError::Busy) | UserError::DirectoryUnavailable => {
            AdminApiError::unavailable(BUSY_RETRY_AFTER)
        }
        other => AdminApiError::internal(other),
    }
}
