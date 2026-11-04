//! Console audit of strategy writes: the guards and the snapshots they record. Values of
//! secret catalog options are masked here; the console audit log masks credential-like
//! option names on top (it errs on the side of masking).

use std::collections::BTreeMap;

use deskhaven_config::REDACTED;
use deskhaven_core::console_audit::TxGuard;
use deskhaven_core::state::AppState;
use deskhaven_core::strategies::catalog;
use deskhaven_core::strategies::{Assignment, Strategy};
use serde::Serialize;

use super::view::{TargetKind, kind_of};
use crate::{Audit, AuditCtx};

/// Snapshot of a strategy in its audit rows.
#[derive(Serialize)]
struct StrategySnapshot<'a> {
    name: &'a str,
    note: &'a str,
    options: BTreeMap<&'a str, &'a str>,
}

fn masked<'a>(key: &'a str, value: &'a str) -> (&'a str, &'a str) {
    let secret = !value.is_empty() && catalog::is_secret(key);
    (key, if secret { REDACTED } else { value })
}

fn snapshot(s: &Strategy) -> StrategySnapshot<'_> {
    StrategySnapshot {
        name: &s.name,
        note: &s.note,
        options: s.options.iter().map(|(k, v)| masked(k, v)).collect(),
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
                audit = audit.before(&snapshot(s));
            }
            if let Some(s) = after {
                audit = audit.after(&snapshot(s));
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
    use deskhaven_core::strategies::Options;
    use deskhaven_db::entity::managed_by::ManagedBy;
    use deskhaven_db::time;

    use super::*;

    #[test]
    fn secret_option_values_are_masked_and_empty_ones_kept() {
        let now = time::now();
        let options = Options::from([
            ("proxy-password".to_owned(), "pw".to_owned()),
            ("preset-address-book-password".to_owned(), String::new()),
            ("whitelist".to_owned(), "10.0.0.0/8".to_owned()),
        ]);
        let strategy = Strategy {
            id: 1,
            name: "s".to_owned(),
            options,
            options_corrupt: false,
            note: String::new(),
            managed_by: ManagedBy::Ui,
            created_at: now,
            updated_at: now,
        };

        let json = serde_json::to_value(snapshot(&strategy)).unwrap();

        assert_eq!(json["options"]["proxy-password"], REDACTED);
        assert_eq!(json["options"]["preset-address-book-password"], "");
        assert_eq!(json["options"]["whitelist"], "10.0.0.0/8");
    }
}
