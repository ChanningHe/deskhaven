//! Linking an OIDC identity to the signed-in account (M4-09, over
//! `deskhaven_core::oidc` `link`):
//!
//! 1. `POST /me/identities/link {provider, current_password, code?}`: re-authentication
//!    first, as one attempt of the login throttle (see `reauth`): the password, and while
//!    TOTP is on also a current code or an unused recovery code (spent). Linking makes the
//!    identity a way in that asks for no local second factor, so it needs what turning
//!    the factor off needs. LDAP accounts are refused (`oidc_failed`,
//!    `directory_account`); OIDC accounts have no password to re-authenticate with
//!    (409 `not_local_account`). Then a `link` flow, its authorization URL, and its browser
//!    binding in the same HttpOnly cookie a console login uses.
//! 2. The IdP redirects to the callback, which sends a link flow to the console's
//!    `/me/oidc-link`. That page posts the IdP's answer to `POST /me/identities/link/verify`
//!    (`link_confirm`): the code is exchanged once and the page shows the verified IdP
//!    identity and the account, with a button.
//! 3. `POST /me/identities/link/complete {state}` links exactly that identity to the
//!    session's account, only in the browser holding the binding and only for the account
//!    the flow was started for (anything else: `oidc_failed`, `expired`). Audited
//!    `user.identity_link` in the linking transaction.
//!
//! Refusals are 403 `oidc_failed` with `details.reason`: `denied`, `unavailable`,
//! `invalid_token`, `identity_taken` (another account's), `provider_linked` (the account
//! already has an identity there), `directory_account`, `account_inactive`, `expired`.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};

use deskhaven_core::oidc::{FLOW_TTL, LinkFailure, OidcError};
use deskhaven_core::state::AppState;
use deskhaven_core::tfa::TfaError;
use deskhaven_core::users::{AuthSource, Password, User};
use garde::Validate;
use serde::Deserialize;
use serde_json::json;
use utoipa::ToSchema;

use super::reauth::{
    Failed, MAX_CODE_INPUT, MAX_PASSWORD_INPUT, not_local, password_failure, throttled, user_error,
};
use super::totp::tfa_error;
use crate::audit::Audit;
use crate::auth::{OIDC_BINDING_COOKIE, OidcStart};
use crate::domain_error::invalid_field;
use crate::session::{Client, ConsoleUser, http_only_cookie};
use crate::transport::SecureTransport;

use crate::{AdminApiError, ErrorCode, ValidJson};

/// Longest accepted provider name or state (characters).
const MAX_FIELD: usize = 2048;

/// Which provider to link, and the re-authentication. Never logged.
#[derive(Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OidcLinkStart {
    /// The provider's name (as `GET /auth/oidc/providers` lists it).
    #[garde(length(chars, min = 1, max = MAX_FIELD))]
    pub provider: String,
    /// The account's current password.
    #[garde(length(chars, min = 1, max = MAX_PASSWORD_INPUT))]
    #[schema(format = Password)]
    pub current_password: String,
    /// Required while TOTP is on: a current TOTP code or an unused recovery code.
    #[garde(inner(length(chars, min = 1, max = MAX_CODE_INPUT)))]
    pub code: Option<String>,
}

/// Starts linking an OIDC identity to the signed-in account (see the module docs).
#[utoipa::path(
    post,
    path = "/me/identities/link",
    operation_id = "start_oidc_link",
    tag = "me",
    security(("console_session" = ["self"])),
    request_body = OidcLinkStart,
    responses(
        (status = 200, description = "The authorization URL; the binding cookie is set", body = OidcStart),
        (status = 403, description = "The account cannot link (`oidc_failed`, `details.reason`), plain-HTTP console (`insecure_transport`), or `csrf_failed`", body = crate::ErrorEnvelope),
        (status = 404, description = "Unknown or disabled provider, or OIDC not configured (`not_found`)", body = crate::ErrorEnvelope),
        (status = 409, description = "An OIDC account: no password to re-authenticate with (`not_local_account`)", body = crate::ErrorEnvelope),
        (status = 422, description = "Wrong password (`wrong_password`), wrong code (`invalid_code`), or a missing field (`validation_failed`)", body = crate::ErrorEnvelope),
        (status = 429, description = "Too many attempts (`rate_limited`)", body = crate::ErrorEnvelope),
        (status = 503, description = "The provider or the directory is unreachable, or password hashing saturated (`unavailable`)", body = crate::ErrorEnvelope),
    )
)]
pub async fn start(
    State(state): State<Arc<AppState>>,
    _secure: SecureTransport,
    ConsoleUser(principal): ConsoleUser,
    Client(client): Client,
    ValidJson(body): ValidJson<OidcLinkStart>,
) -> Result<Response, AdminApiError> {
    let user = state
        .users
        .get(principal.user_id)
        .await
        .map_err(user_error)?;
    match user.auth_source {
        AuthSource::Ldap => return Err(link_failed(LinkFailure::DirectoryAccount.code())),
        AuthSource::Oidc => return Err(not_local()),
        AuthSource::Local => {}
    }
    let code = match (user.totp_enabled, body.code.as_deref()) {
        (true, None) => return Err(invalid_field("code", "required")),
        (true, Some(code)) => Some(code.trim()),
        (false, _) => None,
    };
    // Refusals first: an unknown provider or a refused account spends no code.
    state
        .oidc
        .check_link(&body.provider, user.id)
        .await
        .map_err(start_error)?;
    let password = Password::new(body.current_password.as_str());
    let verify = reauthenticate(&state, &user, &password, code);
    throttled(&state, client, &user.username, verify).await?;
    let started = state
        .oidc
        .begin_link(&body.provider, user.id)
        .await
        .map_err(start_error)?;
    let cookie = http_only_cookie(OIDC_BINDING_COOKIE, started.binding.expose(), FLOW_TTL);
    let audit = Audit::new("user.identity_link_begin")
        .target("user", user.id)
        .after(&json!({ "provider": body.provider }));
    let reply = Json(OidcStart {
        url: started.url.to_string(),
    });
    Ok(([(header::SET_COOKIE, cookie)], audit, reply).into_response())
}

/// The password, then (TOTP on) the code, spent: one throttle attempt.
async fn reauthenticate(
    state: &AppState,
    user: &User,
    password: &Password,
    code: Option<&str>,
) -> Result<(), Failed> {
    let checked = state.ldap.verify_password(user.id, password).await;
    checked.map_err(password_failure)?;
    let Some(code) = code else {
        return Ok(());
    };
    state
        .tfa
        .prove_possession(user.id, code)
        .await
        .map_err(|err| match err {
            TfaError::InvalidCode => {
                Failed::Counted(AdminApiError::new(ErrorCode::InvalidCode, "wrong code"))
            }
            other => Failed::Refunded(tfa_error(other)),
        })
}

fn start_error(err: OidcError) -> AdminApiError {
    match err {
        OidcError::NotConfigured | OidcError::UnknownProvider => {
            AdminApiError::not_found("unknown OIDC provider")
        }
        OidcError::Unavailable(_) => {
            AdminApiError::new(ErrorCode::Unavailable, "the OIDC provider is unavailable")
        }
        OidcError::LinkRefused(failure) => link_failed(failure.code()),
        other => AdminApiError::internal(other),
    }
}

pub(super) fn link_failed(reason: &'static str) -> AdminApiError {
    AdminApiError::new(ErrorCode::OidcFailed, "the OIDC identity was not linked")
        .with_details(json!({ "reason": reason }))
}
