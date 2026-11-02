//! TOTP enrolment of the signed-in user (06 §3.3) over `TfaService`:
//!
//! 1. `POST /me/totp/enrolment`: a new pending seed (replacing an earlier pending one),
//!    shown once as base32 text, `otpauth://` URI and QR path data (`qr`). 409 while TOTP
//!    is enabled.
//! 2. `POST /me/totp/enrolment/confirm {code}`: a code of the pending seed enables TOTP
//!    and issues the recovery codes, returned once. Then every console session of the user
//!    is rotated (`SessionService::rotate_console`): this one is replaced by a fresh one
//!    (cookies), others are revoked, so no session that never passed the new factor keeps
//!    working with it (a session restricted to this enrolment included). Wrong code: 422
//!    `invalid_code`.
//!
//! Both are recorded in their own transaction (`user.totp_enrolment_begin`,
//! `user.totp_enable`). Responses carrying secrets are `Cache-Control: no-store`.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use deskhaven_core::state::AppState;
use deskhaven_core::tfa::TfaError;
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::qr::{self, QrCode};
use crate::audit::{Audit, AuditCtx, Audited};
use crate::auth::{user_agent, with_session};
use crate::me::Me;
use crate::session::{Client, ConsoleUser};
use crate::{AdminApiError, ErrorCode, ValidJson};

/// Longest accepted code input (characters).
const MAX_CODE_INPUT: usize = 16;

/// A pending enrolment, shown once.
#[derive(Debug, Serialize, ToSchema)]
pub struct TotpEnrolment {
    /// The seed in base32, for manual entry.
    pub secret: String,
    /// `otpauth://` provisioning URI.
    pub uri: String,
    /// The URI as a QR code.
    pub qr: QrCode,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TotpConfirm {
    /// A current six-digit code of the pending seed.
    #[garde(length(chars, min = 1, max = MAX_CODE_INPUT))]
    pub code: String,
}

/// TOTP is enabled: the recovery codes (shown once) and the user.
#[derive(Debug, Serialize, ToSchema)]
pub struct TotpEnabled {
    pub recovery_codes: Vec<String>,
    pub user: Me,
}

/// Starts a TOTP enrolment.
#[utoipa::path(
    post,
    path = "/me/totp/enrolment",
    operation_id = "begin_totp_enrolment",
    tag = "me",
    security(("console_session" = ["session"])),
    responses(
        (status = 200, description = "The pending seed", body = TotpEnrolment),
        (status = 409, description = "TOTP is already enabled (`conflict`)", body = crate::ErrorEnvelope),
    )
)]
pub async fn begin(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
    ctx: AuditCtx,
) -> Result<Response, AdminApiError> {
    let user_id = principal.user_id;
    let audit = ctx.entry(Audit::new("user.totp_enrolment_begin").target("user", user_id));
    let enrolment = state
        .tfa
        .begin_enrolment_audited(user_id, &audit)
        .await
        .map_err(tfa_error)?;
    let qr = qr::encode(&enrolment.uri)
        .ok_or_else(|| AdminApiError::internal("the provisioning URI does not fit a QR code"))?;
    let body = TotpEnrolment {
        secret: enrolment.secret.to_string(),
        uri: enrolment.uri.to_string(),
        qr,
    };
    Ok((no_store(), Audited, Json(body)).into_response())
}

/// Confirms the TOTP enrolment with a code.
#[utoipa::path(
    post,
    path = "/me/totp/enrolment/confirm",
    operation_id = "confirm_totp_enrolment",
    tag = "me",
    security(("console_session" = ["session"])),
    request_body = TotpConfirm,
    responses(
        (status = 200, description = "Enabled; recovery codes; a fresh session (cookies set)", body = TotpEnabled),
        (status = 409, description = "Already enabled, or no enrolment pending (`conflict`)", body = crate::ErrorEnvelope),
        (status = 422, description = "Wrong code (`invalid_code`)", body = crate::ErrorEnvelope),
    )
)]
pub async fn confirm(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
    Client(client): Client,
    ctx: AuditCtx,
    headers: HeaderMap,
    ValidJson(body): ValidJson<TotpConfirm>,
) -> Result<Response, AdminApiError> {
    let user_id = principal.user_id;
    let audit = ctx.entry(Audit::new("user.totp_enable").target("user", user_id));
    let codes = state
        .tfa
        .confirm_enrolment_audited(user_id, body.code.trim(), &audit)
        .await
        .map_err(tfa_error)?;
    let user = state
        .users
        .get(user_id)
        .await
        .map_err(AdminApiError::internal)?;
    let enabled = TotpEnabled {
        recovery_codes: codes.codes().map(str::to_owned).collect(),
        user: Me::of(&state, user),
    };
    let rotated = state
        .sessions
        .rotate_console(
            principal.session_id,
            Some(client.addr),
            user_agent(&headers),
        )
        .await;
    match rotated {
        Ok((session, token)) => Ok(with_session(&session, &token, (Audited, Json(enabled)))),
        Err(err) => {
            // TOTP is on and the codes exist: never swallow them. Without a fresh session
            // the console signs in again (with the new factor).
            tracing::warn!(user_id, error = %err, "TOTP enabled, but the console session could not be rotated");
            Ok((no_store(), Audited, Json(enabled)).into_response())
        }
    }
}

fn no_store() -> [(header::HeaderName, HeaderValue); 1] {
    [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))]
}

fn tfa_error(err: TfaError) -> AdminApiError {
    match err {
        TfaError::AlreadyEnabled => {
            AdminApiError::conflict("two-factor authentication is already enabled")
        }
        TfaError::NoPendingEnrolment => AdminApiError::conflict("no enrolment is pending"),
        TfaError::InvalidCode => AdminApiError::new(ErrorCode::InvalidCode, "wrong code"),
        TfaError::NotFound => AdminApiError::unauthorized(),
        other => AdminApiError::internal(other),
    }
}
