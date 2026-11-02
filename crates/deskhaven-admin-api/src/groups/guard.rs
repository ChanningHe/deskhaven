//! Audit guards of group writes: any admin may manage groups, so beyond re-checking that
//! the actor is still an active admin or owner ([`AuditCtx::admin_guard`]) they never refuse.

use deskhaven_core::console_audit::TxGuard;
use deskhaven_core::groups::{Group, Membership};
use deskhaven_core::state::AppState;
use serde_json::json;

use super::handlers::GroupSnapshot;
use crate::{Audit, AuditCtx};

pub(crate) fn group_guard(
    state: &AppState,
    ctx: &AuditCtx,
    action: &'static str,
) -> TxGuard<Group> {
    let entry_ctx = ctx.clone();
    ctx.admin_guard(
        &state.console_audit,
        move |_, before: Option<&Group>, after: Option<&Group>| {
            let ctx = entry_ctx;
            let id = after.or(before).map_or(0, |g| g.id);
            let mut audit = Audit::new(action).target("group", id);
            if let Some(before) = before {
                audit = audit.before(&GroupSnapshot::from(before));
            }
            if let Some(after) = after {
                audit = audit.after(&GroupSnapshot::from(after));
            }
            Ok(ctx.entry(audit))
        },
    )
}

/// `before` / `after`: `{user_id, username}` when the user was / is a member.
pub(crate) fn member_guard(
    state: &AppState,
    ctx: &AuditCtx,
    action: &'static str,
    group_id: i64,
) -> TxGuard<Membership> {
    let entry_ctx = ctx.clone();
    ctx.admin_guard(
        &state.console_audit,
        move |_, before: Option<&Membership>, after: Option<&Membership>| {
            let ctx = entry_ctx;
            let member = |m: &Membership| json!({ "user_id": m.user_id, "username": m.username });
            let mut audit = Audit::new(action).target("group", group_id);
            if let Some(before) = before {
                audit = audit.before(&member(before));
            }
            if let Some(after) = after {
                audit = audit.after(&member(after));
            }
            Ok(ctx.entry(audit))
        },
    )
}
