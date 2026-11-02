//! The per-route gate every admin operation runs behind, in order: console session
//! (401) -> CSRF header on state-changing methods (403 `csrf_failed`) -> role against
//! the route's [`Permission`] (403) -> handler -> console audit of a successful write.
//! `Public` routes pass straight through.

use std::sync::Arc;

use axum::extract::Request;
use axum::http::Method;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use deskhaven_core::state::AppState;

use crate::AdminApiError;
use crate::audit::{self, Audit, AuditContext};
use crate::permission::Permission;
use crate::session::{self, ConsoleUser};

/// What the gate knows about the route it guards.
#[derive(Clone, Debug)]
pub(crate) struct RouteGuard {
    pub permission: Permission,
    /// Operation id per method: the audit action when the handler names none.
    pub operations: Arc<[(Method, String)]>,
}

impl RouteGuard {
    fn operation_id(&self, method: &Method) -> &str {
        self.operations
            .iter()
            .find(|(m, _)| m == method)
            .map_or("", |(_, id)| id.as_str())
    }
}

pub(crate) async fn gate(
    state: Arc<AppState>,
    guard: RouteGuard,
    req: Request,
    next: Next,
) -> Response {
    guarded(&state, &guard, req, next)
        .await
        .unwrap_or_else(IntoResponse::into_response)
}

async fn guarded(
    state: &AppState,
    guard: &RouteGuard,
    req: Request,
    next: Next,
) -> Result<Response, AdminApiError> {
    if guard.permission == Permission::Public {
        return Ok(next.run(req).await);
    }
    let (mut parts, body) = req.into_parts();
    let client = session::client_addr(&mut parts, &()).await?;
    let token = session::session_token(&parts.headers).ok_or_else(AdminApiError::unauthorized)?;
    let principal = session::authenticate(state, client, token).await?;
    let writes = session::is_state_changing(&parts.method);
    if writes {
        session::check_csrf(&parts.headers, token)?;
    }
    if !guard.permission.allows(principal.role) {
        tracing::debug!(user_id = principal.user_id, permission = ?guard.permission, "role denied");
        return Err(AdminApiError::forbidden());
    }
    let user_id = principal.user_id;
    let method = parts.method.clone();
    parts.extensions.insert(ConsoleUser(principal));

    let mut response = next.run(Request::from_parts(parts, body)).await;
    if !writes || !response.status().is_success() {
        return Ok(response);
    }
    let details = response.extensions_mut().remove::<Audit>();
    let ctx = AuditContext {
        user_id,
        ip: client.addr,
        operation_id: guard.operation_id(&method),
    };
    audit::record(state, ctx, details).await?;
    Ok(response)
}
