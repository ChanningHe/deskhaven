//! Console audit log (04 §7 `console_audit`, 06 §7): who changed what through the console,
//! an API token or the CLI. Append-only from here; retention is the background jobs'
//! business.
//!
//! **Atomicity**: a write operation records its row with [`ConsoleAuditLog::record_in`]
//! on the connection of its own write transaction, so the change and its audit row commit
//! or roll back together. [`ConsoleAuditLog::record`] (own connection) is for events that
//! change nothing in the database, and for the admin API's fallback.
//!
//! Secrets never reach a row. Two layers, because each misses what the other catches:
//! - **Type**: secret values that sit in innocuously named fields (a setting's `value`, a
//!   generic map entry) are [`deskhaven_config::Secret`]s in the snapshot the caller
//!   serializes; they render as `<redacted>` (or `""` when unset).
//! - **Name and shape**: [`redact`] masks members whose key names a secret and strings
//!   that look like credentials (see [`redact`](mod@redact) for the rules), at any depth,
//!   so a plain `String` a caller forgot to wrap is still masked. It errs on the side of
//!   masking: a non-secret key that happens to match (e.g. `api_token_id`) loses its value.
//!
//! `null` stays `null` under either layer: whether a secret is set is not itself secret.

mod guard;
pub mod redact;

use std::net::IpAddr;

use deskhaven_db::entity::console_audit;
use deskhaven_db::time;
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, ConnectionTrait, DatabaseConnection, DbErr};
use serde_json::Value;

pub use deskhaven_db::entity::console_audit::ActorKind;
pub use guard::{ACTOR_INACTIVE, GuardError, Refused, TxGuard};
pub use redact::{is_secret_key, redact};

use crate::clock::SharedClock;

/// Longest stored `action`, `target_type` and `target_id` (characters; longer is cut).
pub const MAX_LABEL_CHARS: usize = 256;

/// Who performed an audited action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Actor {
    User(i64),
    ApiToken(i64),
    Cli,
    System,
}

impl Actor {
    fn columns(self) -> (ActorKind, Option<i64>) {
        match self {
            Self::User(id) => (ActorKind::User, Some(id)),
            Self::ApiToken(id) => (ActorKind::ApiToken, Some(id)),
            Self::Cli => (ActorKind::Cli, None),
            Self::System => (ActorKind::System, None),
        }
    }
}

/// One audited change. `before` / `after` are snapshots of the target (any JSON); they are
/// passed through [`redact`] before they are stored.
#[derive(Debug, Clone, PartialEq)]
pub struct NewConsoleAudit {
    pub actor: Actor,
    /// Stable machine name of the operation, e.g. `user.disable`.
    pub action: String,
    /// Kind of the changed resource, e.g. `user`; empty when there is none.
    pub target_type: String,
    /// Its id; empty when the action has no single target.
    pub target_id: String,
    pub before: Option<Value>,
    pub after: Option<Value>,
    pub ip: Option<IpAddr>,
}

/// Writes console audit rows. Cheap to clone.
#[derive(Clone)]
pub struct ConsoleAuditLog {
    db: DatabaseConnection,
    clock: SharedClock,
}

impl ConsoleAuditLog {
    pub fn new(db: DatabaseConnection, clock: SharedClock) -> Self {
        Self { db, clock }
    }

    /// Inserts one row on the log's own connection; returns its id.
    pub async fn record(&self, entry: &NewConsoleAudit) -> Result<i64, DbErr> {
        self.record_in(&self.db, entry).await
    }

    /// Inserts one row on `conn`, normally the caller's write transaction, so the row
    /// commits (or rolls back) with the change it describes; returns its id.
    pub async fn record_in<C: ConnectionTrait>(
        &self,
        conn: &C,
        entry: &NewConsoleAudit,
    ) -> Result<i64, DbErr> {
        insert(conn, entry, self.clock.now_ms()).await
    }
}

/// Inserts one row on `conn` stamped `now` (unix ms): for services that write a change
/// and its audit row in their own transaction, on their own clock.
pub(crate) async fn insert<C: ConnectionTrait>(
    conn: &C,
    entry: &NewConsoleAudit,
    now: i64,
) -> Result<i64, DbErr> {
    let created_at = time::from_millis(now)
        .ok_or_else(|| DbErr::Custom(format!("timestamp {now} out of range")))?;
    let (actor_kind, actor_id) = entry.actor.columns();
    let row = console_audit::ActiveModel {
        actor_kind: Set(actor_kind),
        actor_id: Set(actor_id),
        action: Set(label(&entry.action)),
        target_type: Set(label(&entry.target_type)),
        target_id: Set(label(&entry.target_id)),
        before: Set(entry.before.as_ref().map(|v| redact(v).to_string())),
        after: Set(entry.after.as_ref().map(|v| redact(v).to_string())),
        ip: Set(entry.ip.map(|ip| ip.to_string())),
        created_at: Set(created_at),
        ..Default::default()
    };
    row.insert(conn).await.map(|row| row.id)
}

fn label(s: &str) -> String {
    s.chars().take(MAX_LABEL_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_bounded() {
        let long = "é".repeat(MAX_LABEL_CHARS + 5);

        assert_eq!(label(&long).chars().count(), MAX_LABEL_CHARS);
        assert_eq!(label("user"), "user");
    }
}
