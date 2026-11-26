//! Re-authentication for self-service changes: a stolen session alone must not take the
//! account over. A wrong password is 422 `wrong_password`, never 401 (the console treats 401
//! as signed out).

use std::future::Future;

use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::state::AppState;
use deskhaven_core::users::{PasswordError, UserError};
use serde_json::json;

use crate::auth::{BUSY_RETRY_AFTER, throttle_error};
use crate::{AdminApiError, ErrorCode};

pub(super) const MAX_PASSWORD_INPUT: usize = 1024;
pub(super) const MAX_CODE_INPUT: usize = 16;

pub(super) async fn with_current_password<T>(
    state: &AppState,
    client: ClientAddr,
    username: &str,
    verify: impl Future<Output = Result<T, UserError>>,
) -> Result<T, AdminApiError> {
    let checked = async { verify.await.map_err(password_failure) };
    throttled(state, client, username, checked).await
}

pub(super) enum Failed {
    Counted(AdminApiError),
    Refunded(AdminApiError),
}

pub(super) fn password_failure(err: UserError) -> Failed {
    match err {
        UserError::WrongPassword => Failed::Counted(AdminApiError::new(
            ErrorCode::WrongPassword,
            "the current password is wrong",
        )),
        other => Failed::Refunded(user_error(other)),
    }
}

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
