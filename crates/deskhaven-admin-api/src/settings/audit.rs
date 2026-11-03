//! The audit guard of settings writes: owner only, one row per batch.
//!
//! `before` is `{changes: [{key, value}]}` of the written keys; `after` is the same (a
//! reset's entry also has `reset: true`) plus the batch's `reason` when given. Values are the public JSON form, so a secret appears only as
//! `{"set": bool}`; the keys are string values here, not object keys, so the core log's
//! name-based masking does not blank `auth.password.min_length`.

use deskhaven_core::console_audit::{Refused, TxGuard};
use deskhaven_core::settings::{AppliedChanges, SettingChange};
use deskhaven_core::state::AppState;
use deskhaven_core::users::Role;
use serde_json::{Value, json};

use crate::domain_error::FORBIDDEN;
use crate::{Audit, AuditCtx};

const ACTION_UPDATE: &str = "setting.update";
const ACTION_RESET: &str = "setting.reset";
const TARGET: &str = "setting";

/// Re-checks, in the write transaction, that the actor is still the owner (the session's
/// role may be stale), then records the batch.
pub(crate) fn settings_guard(state: &AppState, ctx: &AuditCtx) -> TxGuard<AppliedChanges> {
    let entry_ctx = ctx.clone();
    ctx.admin_guard(
        &state.console_audit,
        move |role, before: Option<&AppliedChanges>, after: Option<&AppliedChanges>| {
            if role != Role::Owner {
                return Err(Refused::new(FORBIDDEN));
            }
            let applied = after.or(before);
            let changes = applied.map(|a| a.changes.as_slice()).unwrap_or(&[]);
            let reason = applied.and_then(|a| a.reason.as_deref());
            let all_resets = !changes.is_empty() && changes.iter().all(|c| c.reset);
            let action = if all_resets {
                ACTION_RESET
            } else {
                ACTION_UPDATE
            };
            let audit = Audit::new(action)
                .target(TARGET, target_id(changes))
                .before(&json!({ "changes": entries(changes, before_entry) }))
                .after(&after_snapshot(changes, reason));
            Ok(entry_ctx.entry(audit))
        },
    )
}

fn entries(changes: &[SettingChange], entry: impl Fn(&SettingChange) -> Value) -> Vec<Value> {
    changes.iter().map(entry).collect()
}

fn before_entry(change: &SettingChange) -> Value {
    json!({ "key": change.key, "value": change.before })
}

/// `{changes, reason?}`: the note is only there when the caller gave one.
fn after_snapshot(changes: &[SettingChange], reason: Option<&str>) -> Value {
    let mut snapshot = json!({ "changes": entries(changes, after_entry) });
    if let (Some(reason), Some(members)) = (reason, snapshot.as_object_mut()) {
        members.insert("reason".to_owned(), Value::String(reason.to_owned()));
    }
    snapshot
}

fn after_entry(change: &SettingChange) -> Value {
    match change.reset {
        true => json!({ "key": change.key, "value": change.after, "reset": true }),
        false => json!({ "key": change.key, "value": change.after }),
    }
}

/// The key of a single-key batch; the keys comma-separated otherwise (cut by the log).
fn target_id(changes: &[SettingChange]) -> String {
    let keys: Vec<&str> = changes.iter().map(|c| c.key).collect();
    keys.join(",")
}
