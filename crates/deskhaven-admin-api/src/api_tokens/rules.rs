//! Who may do what with API tokens (06 §4.1), decided inside the write transaction by a
//! [`TxGuard`] together with the audit row.
//!
//! - Any administrator creates tokens; a token carries its creator's authority, which the
//!   core re-checks (active administrator) in the same transaction.
//! - Any administrator revokes tokens of administrators. A token whose creator is an owner
//!   is an owner's authority in someone else's hands, and a system token (no creator) has
//!   no administrator behind it: both need an owner, like every other owner-level
//!   resource (`owner_required`).
//!
//! Snapshots carry the token's identity (id as target, name, prefix, scopes, expiry,
//! creator, revoked flag) and never the secret or its hash. The display prefix is recorded
//! without its `dhk_` scheme marker: the console audit masks every string that looks like
//! a token.

use deskhaven_core::api_tokens::{API_TOKEN_PREFIX, TokenTarget};
use deskhaven_core::console_audit::{ConsoleAuditLog, Refused, TxGuard};
use deskhaven_core::users::Role;
use serde::Serialize;

use crate::domain_error::OWNER_REQUIRED;
use crate::{Audit, AuditCtx};

/// A token in an audit row.
#[derive(Serialize)]
struct TokenSnapshot<'a> {
    name: &'a str,
    prefix: &'a str,
    scopes: Vec<&'static str>,
    created_by: Option<i64>,
    expires_at: Option<i64>,
    revoked: bool,
}

impl<'a> From<&'a TokenTarget> for TokenSnapshot<'a> {
    fn from(target: &'a TokenTarget) -> Self {
        let token = &target.token;
        let prefix = &token.token_prefix;
        Self {
            name: &token.name,
            prefix: prefix.strip_prefix(API_TOKEN_PREFIX).unwrap_or(prefix),
            scopes: token.scopes.iter().map(|s| s.as_str()).collect(),
            created_by: token.created_by,
            expires_at: token.expires_at.map(|at| at.timestamp_millis()),
            revoked: token.revoked_at.is_some(),
        }
    }
}

/// A rule on who may write to an existing token: the actor's committed role against it.
pub(crate) type TokenRule = fn(Role, &TokenTarget) -> Result<(), Refused>;

/// Refuses an admin on a token that is an owner's (or nobody's) authority.
pub(crate) fn may_revoke(actor: Role, target: &TokenTarget) -> Result<(), Refused> {
    match (actor, target.creator_role) {
        (Role::Owner, _) => Ok(()),
        (_, Some(Role::Owner) | None) => Err(Refused::new(OWNER_REQUIRED)),
        (_, Some(Role::Admin | Role::User)) => Ok(()),
    }
}

/// A guard recording `action` on the token. Creation has no rule beyond the actor being
/// an administrator (`admin_guard`); a write to an existing token passes `rule`, which
/// judges the actor's committed role against the token as it was.
pub(crate) fn token_guard(
    ctx: &AuditCtx,
    log: &ConsoleAuditLog,
    action: &'static str,
    rule: Option<TokenRule>,
) -> TxGuard<TokenTarget> {
    let entry_ctx = ctx.clone();
    ctx.admin_guard(
        log,
        move |role, before: Option<&TokenTarget>, after: Option<&TokenTarget>| {
            if let (Some(rule), Some(target)) = (rule, before) {
                rule(role, target)?;
            }
            let id = after.or(before).map_or(0, |t| t.token.id);
            let mut audit = Audit::new(action).target("api_token", id);
            if let Some(target) = before {
                audit = audit.before(&TokenSnapshot::from(target));
            }
            if let Some(target) = after {
                audit = audit.after(&TokenSnapshot::from(target));
            }
            Ok(entry_ctx.entry(audit))
        },
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::collections::BTreeSet;

    use deskhaven_core::api_tokens::{ApiToken, Scope};
    use deskhaven_db::time;

    use super::*;

    fn target(creator_role: Option<Role>) -> TokenTarget {
        TokenTarget {
            token: ApiToken {
                id: 1,
                name: "t".to_owned(),
                token_prefix: "dhk_abcdefgh".to_owned(),
                scopes: BTreeSet::from([Scope::Deploy]),
                created_by: Some(2),
                created_at: time::from_millis(1).unwrap(),
                expires_at: None,
                last_used_at: None,
                revoked_at: None,
            },
            creator_role,
        }
    }

    #[test]
    fn owners_revoke_everything_admins_only_admin_tokens() {
        let cases = [
            (Role::Owner, Some(Role::Owner), true),
            (Role::Owner, None, true),
            (Role::Admin, Some(Role::Admin), true),
            (Role::Admin, Some(Role::Owner), false),
            (Role::Admin, None, false),
        ];

        for (actor, creator, allowed) in cases {
            let got = may_revoke(actor, &target(creator));
            assert_eq!(got.is_ok(), allowed, "{actor:?} on {creator:?}");
        }
    }

    #[test]
    fn the_snapshot_drops_the_scheme_marker_so_the_audit_keeps_it() {
        let target = target(Some(Role::Admin));

        let json = serde_json::to_value(TokenSnapshot::from(&target)).unwrap();

        assert_eq!(json["prefix"], "abcdefgh");
        assert_eq!(json["scopes"], serde_json::json!(["deploy"]));
        assert!(json.get("token_hash").is_none());
    }
}
