//! The audit guard of settings writes: owner only, one row per batch.
//!
//! `before` / `after` are `[{key, value}]` of the written keys (a reset's `after` also has
//! `reset: true`). Values are the public JSON form, so a secret appears only as
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
            let changes = after
                .or(before)
                .map(|a| a.changes.as_slice())
                .unwrap_or(&[]);
            let all_resets = !changes.is_empty() && changes.iter().all(|c| c.reset);
            let action = if all_resets {
                ACTION_RESET
            } else {
                ACTION_UPDATE
            };
            let audit = Audit::new(action)
                .target(TARGET, target_id(changes))
                .before(&entries(
                    changes,
                    |c| json!({ "key": c.key, "value": c.before }),
                ))
                .after(&entries(changes, after_entry));
            Ok(entry_ctx.entry(audit))
        },
    )
}

fn entries(changes: &[SettingChange], entry: impl Fn(&SettingChange) -> Value) -> Vec<Value> {
    changes.iter().map(entry).collect()
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
