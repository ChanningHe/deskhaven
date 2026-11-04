//! `GET /audit/logins`.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use deskhaven_core::audit_query::{AuditQuery, LoginEntry, LoginQuery, parse_stored, stored_name};
use deskhaven_core::state::AppState;
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::filter::{MAX_LABEL_CHARS, MAX_Q_CHARS, WindowQuery, known, trimmed};
use super::view::AuditUserView;
use crate::{AdminApiError, PageQuery, Paginated, ValidQuery};

/// Filters of the login list.
#[derive(Debug, Clone, Default, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct LoginFilter {
    /// The account the attempt matched.
    #[garde(skip)]
    pub user: Option<i64>,
    /// `password`, `totp`, `email_code`, `oidc`, `ldap` or `recovery`.
    #[garde(length(max = 16))]
    #[param(pattern = "^(password|totp|email_code|oidc|ldap|recovery)$")]
    pub method: Option<String>,
    /// `rustdesk`, `console` or `api`.
    #[garde(length(max = 16))]
    #[param(pattern = "^(rustdesk|console|api)$")]
    pub client: Option<String>,
    /// The outcome label exactly, e.g. `success` or `invalid_credentials`.
    #[garde(length(chars, max = MAX_LABEL_CHARS))]
    #[param(max_length = 256)]
    pub result: Option<String>,
    /// The RustDesk id of the client device.
    #[garde(length(chars, max = MAX_LABEL_CHARS))]
    #[param(max_length = 256)]
    pub device: Option<String>,
    /// Substring of the attempted username or address (case-insensitive, literal).
    #[garde(length(chars, max = MAX_Q_CHARS))]
    #[param(max_length = 200)]
    pub q: Option<String>,
}

/// One login attempt. `username_attempted` and `rustdesk_id` are what the client sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct LoginAuditView {
    pub id: i64,
    /// The matched account; `null` for an unknown username or a deleted account.
    pub user: Option<AuditUserView>,
    pub username_attempted: String,
    pub method: String,
    /// Outcome label: `success`, `invalid_credentials`, `locked`, ...
    pub result: String,
    pub client: String,
    pub rustdesk_id: Option<String>,
    pub ip: Option<String>,
    pub created_at: i64,
}

impl From<LoginEntry> for LoginAuditView {
    fn from(entry: LoginEntry) -> Self {
        let row = entry.row;
        Self {
            id: row.id,
            user: entry.user.map(Into::into),
            username_attempted: row.username_attempted,
            method: stored_name(&row.method),
            result: row.result,
            client: stored_name(&row.client),
            rustdesk_id: row.rustdesk_id,
            ip: row.ip,
            created_at: row.created_at.timestamp_millis(),
        }
    }
}

/// Login attempts, newest first by default.
#[utoipa::path(
    get,
    path = "/audit/logins",
    tag = "audit",
    operation_id = "list_login_audit",
    security(("console_session" = ["admin"])),
    params(PageQuery, WindowQuery, LoginFilter),
    responses((status = 200, description = "One page of login attempts", body = Paginated<LoginAuditView>))
)]
pub async fn list_login_audit(
    State(state): State<Arc<AppState>>,
    ValidQuery(page): ValidQuery<PageQuery>,
    ValidQuery(window): ValidQuery<WindowQuery>,
    ValidQuery(filter): ValidQuery<LoginFilter>,
) -> Result<Json<Paginated<LoginAuditView>>, AdminApiError> {
    let query = LoginQuery {
        listing: window.listing(page)?,
        user_id: filter.user,
        method: known("method", filter.method.as_deref(), parse_stored)?,
        client: known("client", filter.client.as_deref(), parse_stored)?,
        result: trimmed(filter.result),
        device: trimmed(filter.device),
        q: filter.q,
    };
    let found = AuditQuery::new(state.db.clone()).logins(&query).await?;
    Ok(Json(Paginated::from_page(found, Into::into)))
}
