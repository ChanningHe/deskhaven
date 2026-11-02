//! The per-route gate every admin operation runs behind, in order:
//! 1. state-changing methods (all routes, `Public` included): same-origin check and the
//!    CSRF header (403 `csrf_failed`; see `origin` and `session`);
//! 2. authenticated routes: console session (401), then role against the route's
//!    [`Permission`] (403);
//! 3. handler;
//! 4. console audit fallback for 2xx responses (see `audit`).
//!
//! The gate wraps the route's whole method router, so a method the route does not serve
//! (405) and `HEAD` (served by the `GET` handler, under the `GET` gate) pass it too.

use std::sync::Arc;

use axum::extract::Request;
use axum::http::Method;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use deskhaven_core::state::AppState;

use crate::AdminApiError;
use crate::audit::{self, Audit, Audited, Fallback};
use crate::origin::{TrustedOrigins, check_same_origin};
use crate::permission::Permission;
use crate::session::{self, ConsoleUser, SessionSlot};

/// What the gate knows about the route it guards.
#[derive(Clone, Debug)]
pub(crate) struct RouteGuard {
    pub permission: Permission,
    /// Operation id per method: the audit action when the handler names none.
    pub operations: Arc<[(Method, String)]>,
    pub origins: TrustedOrigins,
}

impl RouteGuard {
    fn operation_id(&self, method: &Method) -> &str {
        let method = if method == Method::HEAD {
            &Method::GET
        } else {
            method
        };
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
    state: &Arc<AppState>,
    guard: &RouteGuard,
    req: Request,
    next: Next,
) -> Result<Response, AdminApiError> {
    let (mut parts, body) = req.into_parts();
    let writes = session::is_state_changing(&parts.method);
    if writes {
        check_same_origin(&parts.headers, &guard.origins)?;
        session::check_csrf(&parts.headers, session::session_token(&parts.headers))?;
    }
    let client = session::client_addr(&mut parts, &()).await?;
    let slot = SessionSlot::default();
    let gate_user = match guard.permission {
        Permission::Public => {
            parts.extensions.insert(slot.clone());
            None
        }
        permission => {
            let token =
                session::session_token(&parts.headers).ok_or_else(AdminApiError::unauthorized)?;
            let principal = session::authenticate(state, client, token).await?;
            if !permission.allows(principal.role) {
                tracing::debug!(user_id = principal.user_id, ?permission, "role denied");
                return Err(AdminApiError::forbidden());
            }
            let user_id = principal.user_id;
            parts.extensions.insert(ConsoleUser(principal));
            Some(user_id)
        }
    };
    let method = parts.method.clone();

    let mut response = next.run(Request::from_parts(parts, body)).await;
    // No await from here on: the fallback task must exist before the gate can be dropped.
    let audited = response.extensions_mut().remove::<Audited>().is_some();
    let attached = response.extensions_mut().remove::<Audit>();
    if audited || !response.status().is_success() {
        return Ok(response);
    }
    // A `Public` handler that authenticated through `ConsoleUser` is attributed too.
    let user_id = gate_user.or_else(|| slot.0.get().copied());
    let audit = match attached {
        Some(audit) => audit,
        None if writes && gate_user.is_some() => Audit::default(),
        None => return Ok(response),
    };
    let ctx = Fallback {
        user_id,
        ip: client.addr,
        operation_id: guard.operation_id(&method),
    };
    audit::record_detached(state, ctx, audit);
    Ok(response)
}
