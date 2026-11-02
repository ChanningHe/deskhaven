//! Test-only routes exercising the machinery: one per permission, a write recorded in its
//! own transaction, fallback-audited writes and reads, a failing write, validated input,
//! a paginated list and a path whose methods have different permissions.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use deskhaven_admin_api::{
    AdminApiError, AdminRoutes, Audit, AuditCtx, Audited, ConsoleUser, PageQuery, Paginated,
    ValidJson, ValidQuery,
};
use deskhaven_config::Secret;
use deskhaven_core::console_audit::Actor;
use deskhaven_core::pagination::Page;
use deskhaven_core::state::AppState;
use deskhaven_db::entity::group;
use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::{begin_write, time};
use garde::Validate;
use sea_orm::ActiveModelTrait;
use sea_orm::ActiveValue::Set;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Notify;
use utoipa::ToSchema;
use utoipa_axum::routes;

/// Notified by [`fallback_write`] just before it returns.
pub static FALLBACK_HANDLER_DONE: Notify = Notify::const_new();
/// Actor [`public_write`] names for itself.
pub const PUBLIC_ACTOR: Actor = Actor::User(424_242);

#[derive(Serialize)]
struct Snapshot {
    name: &'static str,
    /// A secret in an innocuously named field: masked by the type.
    value: Secret,
    /// A plain string in a secret-named field: masked by the name.
    password: &'static str,
}

/// Fallback-audited admin write with before/after snapshots holding secrets.
#[utoipa::path(post, path = "/probe/admin", security(("console_session" = ["admin"])), responses((status = 200)))]
async fn admin_write() -> (Audit, Json<Value>) {
    let before = Snapshot {
        name: "old",
        value: Secret::new("old-secret"),
        password: "hunter2",
    };
    let after = Snapshot {
        name: "new",
        value: Secret::new("new-secret"),
        password: "hunter3",
    };
    let audit = Audit::new("probe.update")
        .target("probe", 7)
        .before(&before)
        .after(&after);
    (audit, Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AtomicBody {
    #[garde(length(chars, min = 1, max = 32))]
    pub name: String,
    /// Fail after the change and the audit row were written, before commit.
    #[garde(skip)]
    pub fail: bool,
}

/// The production pattern: the change and its audit row in one transaction.
#[utoipa::path(post, path = "/probe/atomic", security(("console_session" = ["admin"])), request_body = AtomicBody, responses((status = 200)))]
async fn atomic_write(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidJson(body): ValidJson<AtomicBody>,
) -> Result<(Audited, Json<Value>), AdminApiError> {
    let tx = begin_write(&state.db)
        .await
        .map_err(AdminApiError::internal)?;
    let now = time::now();
    let created = group::ActiveModel {
        name: Set(body.name.clone()),
        note: Set(String::new()),
        managed_by: Set(ManagedBy::Ui),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&tx)
    .await
    .map_err(AdminApiError::internal)?;
    let audit = Audit::new("group.create")
        .target("group", created.id)
        .after(&json!({ "name": body.name }));
    state
        .console_audit
        .record_in(&tx, &ctx.entry(audit))
        .await
        .map_err(AdminApiError::internal)?;
    if body.fail {
        return Err(AdminApiError::conflict("failed before commit"));
    }
    tx.commit().await.map_err(AdminApiError::internal)?;
    Ok((Audited, Json(json!({ "id": created.id }))))
}

/// A fallback-audited write that signals when the handler is done.
#[utoipa::path(post, path = "/probe/fallback", security(("console_session" = ["admin"])), responses((status = 200)))]
async fn fallback_write() -> (Audit, Json<Value>) {
    FALLBACK_HANDLER_DONE.notify_one();
    (Audit::new("probe.fallback"), Json(json!({ "ok": true })))
}

/// A read that reveals a secret and audits it.
#[utoipa::path(get, path = "/probe/secret", security(("console_session" = ["admin"])), responses((status = 200)))]
async fn secret_view() -> (Audit, Json<Value>) {
    let audit = Audit::new("probe.secret.view").target("probe", 1);
    (audit, Json(json!({ "secret": "shown once" })))
}

/// Owner-only write that supplies no audit details.
#[utoipa::path(delete, path = "/probe/owner", security(("console_session" = ["owner"])), responses((status = 200)))]
async fn owner_write() -> Json<Value> {
    Json(json!({ "ok": true }))
}

/// An admin write that fails.
#[utoipa::path(put, path = "/probe/failing", security(("console_session" = ["admin"])), responses((status = 409)))]
async fn failing_write() -> Result<Json<Value>, AdminApiError> {
    Err(AdminApiError::conflict("version mismatch"))
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProbeBody {
    #[garde(length(chars, min = 1, max = 8))]
    pub name: String,
    #[garde(range(min = 1, max = 10))]
    pub count: u32,
}

/// Self-service write with a validated body.
#[utoipa::path(post, path = "/probe/self", security(("console_session" = ["self"])), request_body = ProbeBody, responses((status = 200)))]
async fn self_write(ValidJson(body): ValidJson<ProbeBody>) -> Json<Value> {
    Json(json!({ "name": body.name, "count": body.count }))
}

/// No session needed; audits with an actor it names itself (e.g. setup's new owner).
#[utoipa::path(post, path = "/probe/public", security(()), responses((status = 200)))]
async fn public_write() -> (Audit, Json<Value>) {
    let audit = Audit::new("probe.public").actor(PUBLIC_ACTOR);
    (audit, Json(json!({ "ok": true })))
}

/// No session needed, but uses it when there is one (e.g. logout); audits as that user.
#[utoipa::path(post, path = "/probe/public-session", security(()), responses((status = 200)))]
async fn public_session(ConsoleUser(user): ConsoleUser) -> (Audit, Json<Value>) {
    let audit = Audit::new("probe.public.session");
    (audit, Json(json!({ "user_id": user.user_id })))
}

/// One path, two permissions: GET for any console user ...
#[utoipa::path(get, path = "/probe/split", security(("console_session" = ["self"])), responses((status = 200)))]
async fn split_read() -> Json<Value> {
    Json(json!({ "read": true }))
}

/// ... POST for the owner only.
#[utoipa::path(post, path = "/probe/split", security(("console_session" = ["owner"])), responses((status = 200)))]
async fn split_write() -> Json<Value> {
    Json(json!({ "written": true }))
}

/// A paginated list of 0..total.
#[utoipa::path(get, path = "/probe/list", security(("console_session" = ["self"])), params(PageQuery), responses((status = 200, body = Paginated<u64>)))]
async fn list(ValidQuery(q): ValidQuery<PageQuery>) -> Json<Paginated<u64>> {
    let total = 45;
    let start = (q.page - 1) * q.page_size;
    let page = Page {
        items: (start..total.min(start + q.page_size)).collect(),
        total,
        page: q.page,
        page_size: q.page_size,
    };
    Json(Paginated::from_page(page, |n| n))
}

pub fn routes() -> AdminRoutes {
    let add = |r: AdminRoutes, m| r.routes(m).unwrap();
    let r = AdminRoutes::new();
    let r = add(r, routes!(admin_write));
    let r = add(r, routes!(atomic_write));
    let r = add(r, routes!(fallback_write));
    let r = add(r, routes!(secret_view));
    let r = add(r, routes!(owner_write));
    let r = add(r, routes!(failing_write));
    let r = add(r, routes!(self_write));
    let r = add(r, routes!(public_write));
    let r = add(r, routes!(public_session));
    let r = add(r, routes!(split_read));
    let r = add(r, routes!(split_write));
    add(r, routes!(list))
}
