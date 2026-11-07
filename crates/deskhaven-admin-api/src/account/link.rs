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
//!    `/me/oidc-link`. That page shows `GET /me/identities/link/preview?state=` (which
//!    provider, which account) and a button.
//! 3. `POST /me/identities/link/complete {state, code | error}` links the identity to the
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
use axum::http::{HeaderMap, header};
use axum::response::{AppendHeaders, IntoResponse, Response};
use deskhaven_core::console_audit::TxGuard;
use deskhaven_core::oidc::{
    CallbackError, CallbackParams, FLOW_TTL, IdentityChange, LinkFailure, OidcError,
};
use deskhaven_core::state::AppState;
use deskhaven_core::tfa::TfaError;
use deskhaven_core::users::{AuthSource, Password, User};
use garde::Validate;
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::{IntoParams, ToSchema};

use super::reauth::{
    Failed, MAX_CODE_INPUT, MAX_PASSWORD_INPUT, not_local, password_failure, throttled, user_error,
};
use super::totp::tfa_error;
use crate::audit::{Audit, AuditCtx, Audited};
use crate::auth::{OIDC_BINDING_COOKIE, OidcCompleteRequest, OidcStart, clear_binding_cookie};
use crate::domain_error::invalid_field;
use crate::session::{self, Client, ConsoleUser, http_only_cookie};
use crate::transport::SecureTransport;
use crate::users::view::IdentityView;
use crate::{AdminApiError, ErrorCode, ValidJson, ValidQuery};

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

#[derive(Debug, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct OidcLinkState {
    /// The `state` the identity provider returned.
    #[garde(length(chars, min = 1, max = MAX_FIELD))]
    pub state: String,
}

/// What the confirmation page names.
#[derive(Debug, Serialize, ToSchema)]
pub struct OidcLinkPreview {
    /// The provider's display name.
    pub provider: String,
    /// The account the identity will be linked to (the session's).
    pub username: String,
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

/// What the console's confirmation page shows for a link flow of this browser and account.
#[utoipa::path(
    get,
    path = "/me/identities/link/preview",
    operation_id = "preview_oidc_link",
    tag = "me",
    security(("console_session" = ["self"])),
    params(OidcLinkState),
    responses(
        (status = 200, description = "The provider and the account", body = OidcLinkPreview),
        (status = 403, description = "No live link flow of this browser and account (`oidc_failed`, reason `expired`)", body = crate::ErrorEnvelope),
    )
)]
pub async fn preview(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
    headers: HeaderMap,
    ValidQuery(query): ValidQuery<OidcLinkState>,
) -> Result<Json<OidcLinkPreview>, AdminApiError> {
    let binding = session::cookie(&headers, OIDC_BINDING_COOKIE);
    let shown = state
        .oidc
        .preview_link(Some(&query.state), binding, principal.user_id)
        .await
        .map_err(callback_error)?;
    let user = state
        .users
        .get(principal.user_id)
        .await
        .map_err(user_error)?;
    Ok(Json(OidcLinkPreview {
        provider: shown.provider,
        username: user.username,
    }))
}

/// Completes the link in the browser that started it (see the module docs).
#[utoipa::path(
    post,
    path = "/me/identities/link/complete",
    operation_id = "complete_oidc_link",
    tag = "me",
    security(("console_session" = ["self"])),
    request_body = OidcCompleteRequest,
    responses(
        (status = 200, description = "Linked; the binding cookie is cleared", body = IdentityView),
        (status = 403, description = "Not linked (`oidc_failed`, `details.reason`), plain-HTTP console (`insecure_transport`), or `csrf_failed`", body = crate::ErrorEnvelope),
    )
)]
pub async fn complete(
    State(state): State<Arc<AppState>>,
    _secure: SecureTransport,
    ConsoleUser(principal): ConsoleUser,
    ctx: AuditCtx,
    headers: HeaderMap,
    ValidJson(request): ValidJson<OidcCompleteRequest>,
) -> Result<Response, AdminApiError> {
    let params = CallbackParams {
        state: Some(&request.state),
        code: request.code.as_deref(),
        error: request.error.as_deref(),
    };
    let binding = session::cookie(&headers, OIDC_BINDING_COOKIE);
    let user_id = principal.user_id;
    let guard = link_guard(&state, ctx, user_id);
    let report = state
        .oidc
        .complete_link(&params, binding, user_id, guard)
        .await
        .map_err(callback_error)?;
    let identity = report.outcome.map_err(|f| link_failed(f.code()))?;
    let clear = AppendHeaders([(header::SET_COOKIE, clear_binding_cookie())]);
    Ok((clear, Audited, Json(IdentityView::from(identity))).into_response())
}

/// Records `user.identity_link` in the linking transaction (the account must still be
/// active).
fn link_guard(state: &AppState, ctx: AuditCtx, user_id: i64) -> TxGuard<IdentityChange> {
    TxGuard::for_account(
        &state.console_audit,
        user_id,
        move |_, _, after: Option<&IdentityChange>| {
            let audit = Audit::new("user.identity_link").target("user", user_id);
            let audit = match after {
                Some(change) => audit.after(&IdentityView::from(change.identity.clone())),
                None => audit,
            };
            Ok(ctx.entry(audit))
        },
    )
}

fn callback_error(err: CallbackError) -> AdminApiError {
    match err {
        CallbackError::InvalidState => link_failed("expired"),
        other => AdminApiError::internal(other),
    }
}

fn link_failed(reason: &'static str) -> AdminApiError {
    AdminApiError::new(ErrorCode::OidcFailed, "the OIDC identity was not linked")
        .with_details(json!({ "reason": reason }))
}
