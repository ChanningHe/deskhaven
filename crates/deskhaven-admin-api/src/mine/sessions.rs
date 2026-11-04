//! `GET /me/sessions`, `DELETE /me/sessions/{id}`, `POST /me/sessions/revoke-others`: the
//! user's own client and console sessions. A session is matched by its owner in the
//! revoking statement itself (`SessionService::revoke_own_audited`), so another user's id
//! is answered like an unknown one (404). Revoking the current console session signs the
//! user out: the response clears the cookies. Audited as `session.revoke` and
//! `session.revoke_others`, in the revocation's transaction.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{AppendHeaders, IntoResponse, Response};
use deskhaven_core::sessions::{Session, SessionKind};
use deskhaven_core::state::AppState;
use serde::Serialize;
use serde_json::Value;
use utoipa::ToSchema;

use crate::audit::{Audit, AuditCtx, Audited};
use crate::session::{ConsoleUser, clear_session_cookies};
use crate::users::view::SessionKindView;
use crate::{AdminApiError, ValidPath};

/// One of the user's usable sessions. Carries no token. The device and browser texts are
/// what the other end reported about itself: the console labels them so.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct MySession {
    pub id: i64,
    pub kind: SessionKindView,
    /// The session making this request.
    pub current: bool,
    /// Client sessions: the RustDesk id of the device.
    pub rustdesk_id: Option<String>,
    /// Client sessions, device-reported.
    pub device_name: Option<String>,
    /// Client sessions, device-reported.
    pub device_os: Option<String>,
    /// Console sessions, browser-reported.
    pub user_agent: Option<String>,
    pub ip: Option<String>,
    pub created_at: i64,
    pub last_used_at: i64,
    pub expires_at: i64,
}

impl MySession {
    fn of(session: Session, current_id: i64) -> Self {
        let info = session.device_info.as_ref();
        let text = |key: &str| {
            let value = info?.get(key).and_then(Value::as_str);
            value.map(str::to_owned)
        };
        Self {
            id: session.id,
            current: session.id == current_id,
            kind: match session.kind {
                SessionKind::Client => SessionKindView::Client,
                SessionKind::Console => SessionKindView::Console,
            },
            device_name: text("name"),
            device_os: text("os"),
            user_agent: text("user_agent"),
            rustdesk_id: session.device.map(|d| d.rustdesk_id),
            ip: session.ip.map(|ip| ip.to_string()),
            created_at: session.created_at.timestamp_millis(),
            last_used_at: session.last_used_at.timestamp_millis(),
            expires_at: session.expires_at.timestamp_millis(),
        }
    }
}

/// How many sessions `POST /me/sessions/revoke-others` ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
pub struct MySessionsRevoked {
    pub revoked: u64,
}

/// The user's usable client and console sessions, most recently used first.
#[utoipa::path(
    get,
    path = "/me/sessions",
    operation_id = "list_my_sessions",
    tag = "me",
    security(("console_session" = ["self"])),
    responses((status = 200, description = "The sessions", body = Vec<MySession>))
)]
pub async fn list(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
) -> Result<Json<Vec<MySession>>, AdminApiError> {
    let sessions = state.sessions.list_for_user(principal.user_id).await?;
    let current = principal.session_id;
    let items = sessions.into_iter().map(|s| MySession::of(s, current));
    Ok(Json(items.collect()))
}

/// Revokes one of the user's sessions.
#[utoipa::path(
    delete,
    path = "/me/sessions/{id}",
    operation_id = "revoke_my_session",
    tag = "me",
    security(("console_session" = ["self"])),
    params(("id" = i64, Path, description = "Session id")),
    responses(
        (status = 204, description = "Revoked; for the current session the cookies are cleared"),
        (status = 404, description = "No such session of the user (`not_found`)", body = crate::ErrorEnvelope),
    )
)]
pub async fn revoke(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<i64>,
) -> Result<Response, AdminApiError> {
    let audit = ctx.entry(Audit::new("session.revoke").target("session", id));
    let revoked = state
        .sessions
        .revoke_own_audited(principal.user_id, id, &audit)
        .await?;
    if !revoked {
        return Err(AdminApiError::not_found("session not found"));
    }
    if id != principal.session_id {
        return Ok((Audited, StatusCode::NO_CONTENT).into_response());
    }
    let cookies = clear_session_cookies().map(|value| (header::SET_COOKIE, value));
    Ok((Audited, AppendHeaders(cookies), StatusCode::NO_CONTENT).into_response())
}

/// Revokes every session of the user except the current one.
#[utoipa::path(
    post,
    path = "/me/sessions/revoke-others",
    operation_id = "revoke_my_other_sessions",
    tag = "me",
    security(("console_session" = ["self"])),
    responses((status = 200, description = "How many sessions were revoked", body = MySessionsRevoked))
)]
pub async fn revoke_others(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
    ctx: AuditCtx,
) -> Result<(Audited, Json<MySessionsRevoked>), AdminApiError> {
    let audit = ctx.entry(Audit::new("session.revoke_others").target("user", principal.user_id));
    let revoked = state
        .sessions
        .revoke_others_audited(principal.user_id, principal.session_id, &audit)
        .await?;
    Ok((Audited, Json(MySessionsRevoked { revoked })))
}
