//! Who may manage whom (06 §4.1), evaluated inside the write transaction through a
//! [`TxGuard`], together with the console audit row of the change.
//!
//! - Admins manage users and admins; only an owner may create, change or delete an owner
//!   account, or give anyone the owner role. A guard sees the target before and after the
//!   change, so "is or becomes an owner" covers promotion, demotion and every other action.
//! - Nobody can take away the last active owner (core: `UserError::LastOwner`).
//! - Nobody deletes their own account (checked by the handler: the ids are known up front).
//! - The rules use the actor's role as committed when the write runs, not the session's:
//!   [`AuditCtx::admin_guard`] re-reads the acting account in the transaction. The
//!   handlers' early checks with the session role only save work.
//!
//! Audit snapshots carry no secret: the account view has none, and a password reset is
//! recorded as a `password` member whose value the core log masks anyway.

use deskhaven_core::console_audit::{ConsoleAuditLog, Refused, TxGuard};
use deskhaven_core::sessions::SessionPrincipal;
use deskhaven_core::users::{Role, User};
use serde::Serialize;
use serde_json::{Map, Value};

use super::view::{AuthSourceView, StatusView};
use crate::domain_error::OWNER_REQUIRED;
use crate::{Audit, AuditCtx, RoleView};

/// The administrator making a change: role for the rules, request facts for the audit row.
#[derive(Clone, Debug)]
pub(crate) struct Acting {
    pub user_id: i64,
    pub role: Role,
    pub ctx: AuditCtx,
}

impl Acting {
    pub fn new(principal: &SessionPrincipal, ctx: AuditCtx) -> Self {
        Self {
            user_id: principal.user_id,
            role: principal.role,
            ctx,
        }
    }

    /// Refuses unless this administrator may act on an account that is `target` (06 §4.1).
    pub fn may_manage(&self, target: &User) -> Result<(), Refused> {
        check_target(self.role, Some(target), None)
    }

    /// The guard of `user.update`: the account rules, and with `expected` (an address being
    /// marked verified) the stored address must still be that one (case-insensitively), or
    /// the change is refused with [`EMAIL_CHANGED`].
    pub fn update_guard(&self, log: &ConsoleAuditLog, expected: Option<String>) -> TxGuard<User> {
        let ctx = self.ctx.clone();
        self.ctx.admin_guard(
            log,
            move |role, before: Option<&User>, after: Option<&User>| {
                if let Some(expected) = &expected {
                    let stored = before.and_then(|u| u.email.as_deref());
                    if !stored.is_some_and(|s| s.eq_ignore_ascii_case(expected.trim())) {
                        return Err(Refused::new(EMAIL_CHANGED));
                    }
                }
                check_target(role, before, after)?;
                Ok(ctx.entry(audit_of("user.update", before, after, None)))
            },
        )
    }

    /// A guard applying the account rules and recording `action` on the account, with
    /// `extra` members merged into the `after` snapshot.
    pub fn user_guard(
        &self,
        log: &ConsoleAuditLog,
        action: &'static str,
        extra: Option<Value>,
    ) -> TxGuard<User> {
        let ctx = self.ctx.clone();
        self.ctx.admin_guard(
            log,
            move |role, before: Option<&User>, after: Option<&User>| {
                check_target(role, before, after)?;
                Ok(ctx.entry(audit_of(action, before, after, extra)))
            },
        )
    }
}

/// [`Refused::code`] of an address marked verified that is no longer the stored one.
pub(crate) const EMAIL_CHANGED: &str = "email_changed";

/// The audit row of `action` on an account seen `before` and `after` the change.
fn audit_of(
    action: &'static str,
    before: Option<&User>,
    after: Option<&User>,
    extra: Option<Value>,
) -> Audit {
    let id = after.or(before).map_or(0, |u| u.id);
    let mut audit = Audit::new(action).target("user", id);
    if let Some(before) = before {
        audit = audit.before(&UserSnapshot::from(before));
    }
    if let Some(after) = after {
        audit = audit.after(&with_extra(UserSnapshot::from(after), extra));
    }
    audit
}

pub(crate) fn check_target(
    actor: Role,
    before: Option<&User>,
    after: Option<&User>,
) -> Result<(), Refused> {
    if actor == Role::Owner {
        return Ok(());
    }
    let is_owner = |u: Option<&User>| u.is_some_and(|u| u.role == Role::Owner);
    if is_owner(before) || is_owner(after) {
        return Err(Refused::new(OWNER_REQUIRED));
    }
    Ok(())
}

/// The audited view of an account. `must_change` is not named `must_change_password`: the
/// audit log masks every member whose name contains `pass`.
#[derive(Serialize)]
pub(crate) struct UserSnapshot {
    username: String,
    display_name: String,
    email: Option<String>,
    email_verified: bool,
    auth_source: AuthSourceView,
    role: RoleView,
    status: StatusView,
    must_change: bool,
    totp_enabled: bool,
    note: String,
}

impl From<&User> for UserSnapshot {
    fn from(u: &User) -> Self {
        Self {
            username: u.username.clone(),
            display_name: u.display_name.clone(),
            email: u.email.clone(),
            email_verified: u.email_verified,
            auth_source: u.auth_source.into(),
            role: u.role.into(),
            status: u.status.into(),
            must_change: u.must_change_password,
            totp_enabled: u.totp_enabled,
            note: u.note.clone(),
        }
    }
}

fn with_extra(snapshot: impl Serialize, extra: Option<Value>) -> Value {
    let mut value = serde_json::to_value(snapshot).unwrap_or(Value::Null);
    if let (Value::Object(members), Some(Value::Object(more))) = (&mut value, extra) {
        members.extend(more);
    }
    value
}

/// `{key: value}` for [`Acting::user_guard`]'s `extra`.
pub(crate) fn extra(key: &str, value: impl Into<Value>) -> Option<Value> {
    let mut members = Map::new();
    members.insert(key.to_owned(), value.into());
    Some(Value::Object(members))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_extra_members_are_merged() {
        let merged = with_extra(serde_json::json!({ "a": 1 }), extra("b", 2));
        assert_eq!(merged, serde_json::json!({ "a": 1, "b": 2 }));
    }
}
