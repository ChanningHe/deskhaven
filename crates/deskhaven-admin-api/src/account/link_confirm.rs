//! The console's link confirmation (`/me/oidc-link`, see `link`):
//!
//! - `POST /me/identities/link/verify {state, code | error}`: exchanges the IdP's code
//!   once and answers the verified IdP identity (subject, verified address, preferred
//!   username), the provider and the account; a reload answers the same without asking the
//!   IdP again. Only for the browser holding the flow's binding cookie and the account the
//!   flow was started for. A refusal at the IdP is 403 `oidc_failed` with the reason
//!   (`denied`, `unavailable`, `invalid_token`); no live flow is reason `expired`. Recorded
//!   as `user.identity_link_verify` (which IdP identity was presented; nothing changes).
//! - `POST /me/identities/link/complete {state}`: links exactly the verified identity
//!   (single use), audited `user.identity_link` in the linking transaction; refusals as in
//!   `link`.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::response::{AppendHeaders, IntoResponse, Response};
use deskhaven_core::console_audit::TxGuard;
use deskhaven_core::oidc::{CallbackError, CallbackParams, IdentityChange};
use deskhaven_core::state::AppState;
use garde::Validate;
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;

use super::link::link_failed;
use super::reauth::user_error;
use crate::audit::{Audit, AuditCtx, Audited};
use crate::auth::{OIDC_BINDING_COOKIE, OidcCompleteRequest, clear_binding_cookie};
use crate::session::{self, ConsoleUser};
use crate::transport::SecureTransport;
use crate::users::view::IdentityView;
use crate::{AdminApiError, ValidJson};

/// Longest accepted state (characters).
const MAX_FIELD: usize = 2048;

/// Which link flow to confirm. Never logged.
#[derive(Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OidcLinkConfirm {
    /// The `state` the identity provider returned.
    #[garde(length(chars, min = 1, max = MAX_FIELD))]
    pub state: String,
}

/// What the confirmation page names: the IdP identity and the account it will link to.
#[derive(Debug, Serialize, ToSchema)]
pub struct OidcLinkPreview {
    /// The provider's display name.
    pub provider: String,
    /// The account the identity will be linked to (the session's).
    pub username: String,
    /// The provider's `sub` for the IdP account.
    pub subject: String,
    /// The IdP account's address, when the provider asserts it verified.
    pub email: Option<String>,
    pub preferred_username: Option<String>,
}

/// Verifies the IdP's answer of a link flow of this browser and account (see the module
/// docs).
#[utoipa::path(
    post,
    path = "/me/identities/link/verify",
    operation_id = "verify_oidc_link",
    tag = "me",
    security(("console_session" = ["self"])),
    request_body = OidcCompleteRequest,
    responses(
        (status = 200, description = "The IdP identity, the provider and the account", body = OidcLinkPreview),
        (status = 403, description = "Refused (`oidc_failed`, `details.reason`), plain-HTTP console (`insecure_transport`), or `csrf_failed`", body = crate::ErrorEnvelope),
    )
)]
pub async fn verify(
    State(state): State<Arc<AppState>>,
    _secure: SecureTransport,
    ConsoleUser(principal): ConsoleUser,
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
    let verified = state
        .oidc
        .verify_link(&params, binding, user_id)
        .await
        .map_err(callback_error)?;
    let shown = verified.map_err(|f| link_failed(f.code()))?;
    let user = state.users.get(user_id).await.map_err(user_error)?;
    let audit = Audit::new("user.identity_link_verify")
        .target("user", user_id)
        .after(&json!({ "provider": shown.provider, "subject": shown.subject }));
    let reply = OidcLinkPreview {
        provider: shown.provider,
        username: user.username,
        subject: shown.subject,
        email: shown.email,
        preferred_username: shown.preferred_username,
    };
    Ok((audit, Json(reply)).into_response())
}

/// Links the verified identity in the browser that started the flow (see the module docs).
#[utoipa::path(
    post,
    path = "/me/identities/link/complete",
    operation_id = "complete_oidc_link",
    tag = "me",
    security(("console_session" = ["self"])),
    request_body = OidcLinkConfirm,
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
    ValidJson(request): ValidJson<OidcLinkConfirm>,
) -> Result<Response, AdminApiError> {
    let binding = session::cookie(&headers, OIDC_BINDING_COOKIE);
    let user_id = principal.user_id;
    let guard = link_guard(&state, ctx, user_id);
    let report = state
        .oidc
        .complete_link(Some(&request.state), binding, user_id, guard)
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
