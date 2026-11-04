//! Console audit of strategy writes: the guards and the snapshots they record.
//!
//! Options are recorded as `[{key, value}]`, not as an object keyed by option name: the
//! console audit log masks members whose *name* looks like a credential, which would blind
//! harmless options such as `temporary-password-length`. Secret catalog options are masked
//! here instead; in an `after` snapshot they carry `changed` (whether the value differs from
//! `before`), so a change stays visible without its value.

use deskhaven_config::REDACTED;
use deskhaven_core::console_audit::TxGuard;
use deskhaven_core::state::AppState;
use deskhaven_core::strategies::catalog;
use deskhaven_core::strategies::{Assignment, Options, Strategy};
use serde::Serialize;

use super::view::{TargetKind, kind_of};
use crate::{Audit, AuditCtx};

/// Snapshot of a strategy in its audit rows.
#[derive(Serialize)]
struct StrategySnapshot<'a> {
    name: &'a str,
    note: &'a str,
    options: Vec<OptionEntry<'a>>,
}

#[derive(Serialize)]
struct OptionEntry<'a> {
    key: &'a str,
    value: &'a str,
    /// Secret options of an `after` snapshot: the value differs from `before`.
    #[serde(skip_serializing_if = "Option::is_none")]
    changed: Option<bool>,
}

/// `previous`: the options of `before` when `s` is the `after` snapshot (empty for a
/// creation); `None` for a `before` snapshot.
fn snapshot<'a>(s: &'a Strategy, previous: Option<&Options>) -> StrategySnapshot<'a> {
    let entry = |(key, value): (&'a String, &'a String)| {
        let secret = !value.is_empty() && catalog::is_secret(key);
        OptionEntry {
            key,
            value: if secret { REDACTED } else { value },
            changed: previous
                .filter(|_| secret)
                .map(|old| old.get(key) != Some(value)),
        }
    };
    StrategySnapshot {
        name: &s.name,
        note: &s.note,
        options: s.options.iter().map(entry).collect(),
    }
}

/// Snapshot of an assignment in its audit rows.
#[derive(Serialize)]
struct AssignmentSnapshot {
    strategy_id: i64,
    target_type: TargetKind,
    target_id: Option<i64>,
}

fn assignment_snapshot(a: &Assignment) -> AssignmentSnapshot {
    let (target_type, target_id) = kind_of(a.target);
    AssignmentSnapshot {
        strategy_id: a.strategy_id,
        target_type,
        target_id,
    }
}

/// Any admin may manage strategies: beyond re-checking that the actor is still an admin or
/// owner the guard never refuses. The row targets the strategy.
pub(super) fn strategy_guard(
    state: &AppState,
    ctx: &AuditCtx,
    action: &'static str,
) -> TxGuard<Strategy> {
    let entry_ctx = ctx.clone();
    ctx.admin_guard(
        &state.console_audit,
        move |_, before: Option<&Strategy>, after: Option<&Strategy>| {
            let id = after.or(before).map_or(0, |s| s.id);
            let mut audit = Audit::new(action).target("strategy", id);
            if let Some(s) = before {
                audit = audit.before(&snapshot(s, None));
            }
            if let Some(s) = after {
                let empty = Options::new();
                let previous = before.map_or(&empty, |b| &b.options);
                audit = audit.after(&snapshot(s, Some(previous)));
            }
            Ok(entry_ctx.entry(audit))
        },
    )
}

/// Like [`strategy_guard`] for assignments; the row targets the strategy assigned (or
/// unassigned), the snapshots are the assignment replaced and the new one.
pub(super) fn assignment_guard(
    state: &AppState,
    ctx: &AuditCtx,
    action: &'static str,
) -> TxGuard<Assignment> {
    let entry_ctx = ctx.clone();
    ctx.admin_guard(
        &state.console_audit,
        move |_, before: Option<&Assignment>, after: Option<&Assignment>| {
            let id = after.or(before).map_or(0, |a| a.strategy_id);
            let mut audit = Audit::new(action).target("strategy", id);
            if let Some(a) = before {
                audit = audit.before(&assignment_snapshot(a));
            }
            if let Some(a) = after {
                audit = audit.after(&assignment_snapshot(a));
            }
            Ok(entry_ctx.entry(audit))
        },
    )
}

#[cfg(test)]
mod tests {
    use deskhaven_db::entity::managed_by::ManagedBy;
    use deskhaven_db::time;
    use serde_json::{Value, json};

    use super::*;

    fn strategy(pairs: &[(&str, &str)]) -> Strategy {
        let now = time::now();
        Strategy {
            id: 1,
            name: "s".to_owned(),
            options: pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            options_corrupt: false,
            note: String::new(),
            managed_by: ManagedBy::Ui,
            created_at: now,
            updated_at: now,
        }
    }

    fn entries(snapshot: &StrategySnapshot<'_>) -> Value {
        serde_json::to_value(snapshot).unwrap()["options"].clone()
    }

    #[test]
    fn secrets_are_masked_with_a_change_marker_and_other_names_stay_readable() {
        let before = strategy(&[
            ("proxy-password", "old"),
            ("temporary-password-length", "8"),
        ]);
        let after = strategy(&[
            ("proxy-password", "new"),
            ("preset-address-book-password", ""),
            ("temporary-password-length", "10"),
        ]);

        let shown = entries(&snapshot(&after, Some(&before.options)));

        assert_eq!(
            shown,
            json!([
                { "key": "preset-address-book-password", "value": "" },
                { "key": "proxy-password", "value": REDACTED, "changed": true },
                { "key": "temporary-password-length", "value": "10" },
            ])
        );
        let same = entries(&snapshot(&before, Some(&before.options)));
        assert_eq!(same[0]["changed"], false);
        assert!(
            entries(&snapshot(&before, None))[0]
                .get("changed")
                .is_none()
        );
        // The audit log's name-based redaction leaves this shape alone.
        let stored = deskhaven_core::console_audit::redact(&shown);
        assert_eq!(stored[2]["value"], "10");
    }
}
