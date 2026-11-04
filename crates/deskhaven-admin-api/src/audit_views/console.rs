//! `GET /audit/console`.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use deskhaven_core::audit_query::{
    AuditQuery, ConsoleActor, ConsoleEntry, ConsoleQuery, parse_snapshot, parse_stored, stored_name,
};
use deskhaven_core::state::AppState;
use garde::Validate;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};

use super::filter::{MAX_LABEL_CHARS, MAX_Q_CHARS, WindowQuery, known, plain_text, trimmed};
use crate::settings::any_json;
use crate::{AdminApiError, PageQuery, Paginated, ValidQuery};

/// Filters of the console audit list.
#[derive(Debug, Clone, Default, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ConsoleFilter {
    /// Changes made by this user account (for API tokens use `actor_kind`).
    #[garde(skip)]
    pub user: Option<i64>,
    /// `user`, `api_token`, `cli` or `system`.
    #[garde(length(max = 16))]
    #[param(pattern = "^(user|api_token|cli|system)$")]
    pub actor_kind: Option<String>,
    /// The action name exactly, e.g. `user.disable`.
    #[garde(length(chars, max = MAX_LABEL_CHARS), custom(plain_text))]
    #[param(max_length = 256)]
    pub action: Option<String>,
    /// The kind of the changed resource exactly, e.g. `user`.
    #[garde(length(chars, max = MAX_LABEL_CHARS), custom(plain_text))]
    #[param(max_length = 256)]
    pub target_type: Option<String>,
    /// Substring of the action, target type or target id (case-insensitive, literal).
    #[garde(length(chars, max = MAX_Q_CHARS), custom(plain_text))]
    #[param(max_length = 200)]
    pub q: Option<String>,
}

/// Who made a change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ConsoleActorView {
    /// `user`, `api_token`, `cli` or `system`.
    pub kind: String,
    /// The user or token id; `null` for `cli` and `system`.
    pub id: Option<i64>,
    /// The username or token name; `null` when it is gone or there is none.
    pub name: Option<String>,
}

impl From<ConsoleActor> for ConsoleActorView {
    fn from(actor: ConsoleActor) -> Self {
        Self {
            kind: stored_name(&actor.kind),
            id: actor.id,
            name: actor.name,
        }
    }
}

/// One audited change. `before` and `after` are the snapshots as stored (secrets were
/// masked when they were written); a retention run (`audit.retention.purge`, actor
/// `system`) carries `{kind, deleted, cutoff, batch}` in `after`.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct ConsoleAuditView {
    pub id: i64,
    pub actor: ConsoleActorView,
    pub action: String,
    pub target_type: String,
    /// Empty when the action has no single target.
    pub target_id: String,
    #[schema(schema_with = any_json)]
    pub before: Option<Value>,
    #[schema(schema_with = any_json)]
    pub after: Option<Value>,
    pub ip: Option<String>,
    pub created_at: i64,
}

impl From<ConsoleEntry> for ConsoleAuditView {
    fn from(entry: ConsoleEntry) -> Self {
        let row = entry.row;
        Self {
            id: row.id,
            actor: entry.actor.into(),
            action: row.action,
            target_type: row.target_type,
            target_id: row.target_id,
            before: parse_snapshot(row.before.as_deref()),
            after: parse_snapshot(row.after.as_deref()),
            ip: row.ip,
            created_at: row.created_at.timestamp_millis(),
        }
    }
}

/// Console audit rows, newest first by default.
#[utoipa::path(
    get,
    path = "/audit/console",
    tag = "audit",
    operation_id = "list_console_audit",
    security(("console_session" = ["admin"])),
    params(PageQuery, WindowQuery, ConsoleFilter),
    responses((status = 200, description = "One page of console audit rows", body = Paginated<ConsoleAuditView>))
)]
pub async fn list_console_audit(
    State(state): State<Arc<AppState>>,
    ValidQuery(page): ValidQuery<PageQuery>,
    ValidQuery(window): ValidQuery<WindowQuery>,
    ValidQuery(filter): ValidQuery<ConsoleFilter>,
) -> Result<Json<Paginated<ConsoleAuditView>>, AdminApiError> {
    let query = ConsoleQuery {
        listing: window.listing(page)?,
        user_id: filter.user,
        actor_kind: known("actor_kind", filter.actor_kind.as_deref(), parse_stored)?,
        action: trimmed(filter.action),
        target_type: trimmed(filter.target_type),
        q: filter.q,
    };
    let found = AuditQuery::new(state.db.clone()).console(&query).await?;
    Ok(Json(Paginated::from_page(found, Into::into)))
}
