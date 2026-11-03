//! `GET|PATCH /settings`, `GET /settings/{key}/history`. Owner only: the settings hold the
//! security policy (lockout, sessions, registration, enforced 2FA) and the SMTP login.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use deskhaven_config::settings::{self as registry, Change, Reset};
use deskhaven_core::console_audit::Actor;
use deskhaven_core::settings::{SettingsError, UpdateRequest};
use deskhaven_core::state::AppState;
use serde_json::{Value, json};

use super::audit::settings_guard;
use super::view::{HistoryEntryView, SettingChangeBody, SettingView, SettingsPatch, authors};
use crate::domain_error::{invalid_field, refused};
use crate::{
    AdminApiError, AuditCtx, Audited, ErrorCode, PageQuery, Paginated, ValidJson, ValidPath,
    ValidQuery,
};

type Problem = crate::ErrorEnvelope<'static>;

/// Every setting with its effective value, source, pin state and form schema, in
/// declaration order (grouped by `category`). Secrets are never returned: their value is
/// `{"set": bool}`.
#[utoipa::path(
    get,
    path = "/settings",
    tag = "settings",
    operation_id = "list_settings",
    security(("console_session" = ["owner"])),
    responses((status = 200, description = "All settings", body = Vec<SettingView>))
)]
pub async fn list_settings(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<SettingView>>, AdminApiError> {
    Ok(Json(views(&state, None).await?))
}

/// Changes and resets settings in one all-or-nothing batch (audited `setting.update`, or
/// `setting.reset` when it only resets). Every change names the `version` it was read at:
/// a stored row that moved on makes the batch a 409 `version_conflict` (nothing is
/// written; `details.conflicts` has the current settings of the stale keys). Pinned keys
/// are 409 `setting_pinned`; invalid values and violated cross-key rules are 422 with
/// `details.fields` keyed by setting key. A change that leaves the value as it is writes
/// nothing. Returns all settings, as after the batch.
#[utoipa::path(
    patch,
    path = "/settings",
    tag = "settings",
    operation_id = "update_settings",
    security(("console_session" = ["owner"])),
    request_body = SettingsPatch,
    responses(
        (status = 200, description = "All settings after the batch", body = Vec<SettingView>),
        (status = 409, description = "A key is pinned (`setting_pinned`, `details.keys`) or changed since it was read (`version_conflict`, `details.conflicts`)", body = Problem),
        (status = 422, description = "Unknown key, invalid value, both or neither of `value` and `reset`, or a violated cross-key rule", body = Problem),
    )
)]
pub async fn update_settings(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidJson(body): ValidJson<SettingsPatch>,
) -> Result<(Audited, Json<Vec<SettingView>>), AdminApiError> {
    let (changes, resets) = split(&body.changes)?;
    let actor = match ctx.actor {
        Actor::User(id) => Some(id),
        _ => None,
    };
    let request = UpdateRequest {
        changes,
        actor,
        reason: body
            .reason
            .map(|r| r.trim().to_owned())
            .filter(|r| !r.is_empty()),
    };
    let guard = settings_guard(&state, &ctx);
    match state.settings.update_guarded(request, resets, guard).await {
        Ok(_) => Ok((Audited, Json(views(&state, None).await?))),
        Err(err) => Err(failure(&state, err).await),
    }
}

/// The changes of one setting, newest first. Secrets show only whether they were set.
#[utoipa::path(
    get,
    path = "/settings/{key}/history",
    tag = "settings",
    operation_id = "get_setting_history",
    security(("console_session" = ["owner"])),
    params(("key" = String, Path, description = "Dotted setting key"), PageQuery),
    responses(
        (status = 200, description = "One page of the history", body = Paginated<HistoryEntryView>),
        (status = 404, description = "No such setting", body = Problem),
    )
)]
pub async fn get_setting_history(
    State(state): State<Arc<AppState>>,
    ValidPath(key): ValidPath<String>,
    ValidQuery(page): ValidQuery<PageQuery>,
) -> Result<Json<Paginated<HistoryEntryView>>, AdminApiError> {
    if registry::lookup(&key).is_none() {
        return Err(AdminApiError::not_found("no such setting"));
    }
    // The key exists and the page was validated: any failure here is the server's.
    let found = state
        .settings
        .history(&key, page.into())
        .await
        .map_err(AdminApiError::internal)?;
    Ok(Json(Paginated::from_page(found, HistoryEntryView::from)))
}

/// Settings and resets of a patch, each key once; a change with both or neither of
/// `value` and `reset` is invalid.
fn split(entries: &[SettingChangeBody]) -> Result<(Vec<Change>, Vec<Reset>), AdminApiError> {
    let mut changes = Vec::new();
    let mut resets = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        match (&entry.value, entry.reset) {
            (Some(value), false) => changes.push(Change {
                key: entry.key.clone(),
                value: value.clone(),
                version: entry.version,
            }),
            (None, true) => resets.push(Reset {
                key: entry.key.clone(),
                version: entry.version,
            }),
            _ => {
                let field = format!("changes[{index}]");
                return Err(invalid_field(&field, "give exactly one of value and reset"));
            }
        }
    }
    Ok((changes, resets))
}

/// The current views of `only` (all settings when `None`), with their authors' names.
async fn views(
    state: &AppState,
    only: Option<&[String]>,
) -> Result<Vec<SettingView>, AdminApiError> {
    let all = state.settings.snapshot().views();
    let chosen: Vec<_> = all
        .into_iter()
        .filter(|v| only.is_none_or(|keys| keys.iter().any(|k| k == v.key)))
        .collect();
    let names = state
        .settings
        .authors(authors(&chosen))
        .await
        .map_err(AdminApiError::internal)?;
    chosen
        .into_iter()
        .map(|view| SettingView::new(view, &names))
        .collect()
}

/// The wire error of a failed update. A version conflict carries the current settings of
/// the stale keys (the service refreshed its snapshot from the locked read), so the
/// console can show the difference.
async fn failure(state: &AppState, err: SettingsError) -> AdminApiError {
    match err {
        SettingsError::Pinned { keys } => AdminApiError::new(
            ErrorCode::SettingPinned,
            "settings fixed by the configuration cannot be changed here",
        )
        .with_details(json!({ "keys": keys })),
        SettingsError::VersionConflict { keys } => conflict(state, &keys).await,
        SettingsError::Invalid { problems } => {
            let mut fields = serde_json::Map::new();
            for problem in problems {
                let slot = fields
                    .entry(problem.key)
                    .or_insert_with(|| Value::Array(Vec::new()));
                if let Value::Array(reasons) = slot {
                    reasons.push(Value::String(problem.reason));
                }
            }
            AdminApiError::validation_failed("invalid input")
                .with_details(json!({ "fields": fields }))
        }
        SettingsError::Refused(r) => refused(r),
        SettingsError::Timeout => AdminApiError::unavailable(std::time::Duration::from_secs(1)),
        other => AdminApiError::internal(other),
    }
}

async fn conflict(state: &AppState, keys: &[String]) -> AdminApiError {
    let error = AdminApiError::new(
        ErrorCode::VersionConflict,
        "settings changed since they were read",
    );
    match views(state, Some(keys)).await {
        Ok(current) => error.with_details(json!({ "keys": keys, "conflicts": current })),
        // The 409 stands even when the detail cannot be read; the console reloads.
        Err(_) => error.with_details(json!({ "keys": keys })),
    }
}
