//! Test-only routes exercising the machinery: one per permission, writes with and
//! without audit details, a failing write, validated input and a paginated list.

use axum::Json;
use deskhaven_admin_api::{
    AdminApiError, AdminRoutes, Audit, ConsoleUser, PageQuery, Paginated, ValidJson, ValidQuery,
};
use deskhaven_config::Secret;
use deskhaven_core::pagination::Page;
use garde::Validate;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;
use utoipa_axum::routes;

#[derive(Serialize)]
struct Snapshot {
    name: &'static str,
    /// A secret in an innocuously named field: masked by the type.
    value: Secret,
    /// A plain string in a secret-named field: masked by the name.
    password: &'static str,
}

/// Audited admin write with before/after snapshots holding secrets.
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

/// No session needed.
#[utoipa::path(post, path = "/probe/public", security(()), responses((status = 200)))]
async fn public_write() -> Json<Value> {
    Json(json!({ "ok": true }))
}

/// A public route that still wants the session if there is one (e.g. logout).
#[utoipa::path(post, path = "/probe/public-session", security(()), responses((status = 200)))]
async fn public_session(ConsoleUser(user): ConsoleUser) -> Json<Value> {
    Json(json!({ "user_id": user.user_id }))
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
    let r = add(r, routes!(owner_write));
    let r = add(r, routes!(failing_write));
    let r = add(r, routes!(self_write));
    let r = add(r, routes!(public_write));
    let r = add(r, routes!(public_session));
    add(r, routes!(list))
}
