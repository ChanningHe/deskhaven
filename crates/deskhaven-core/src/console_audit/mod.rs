//! A write records its row on its own transaction's connection, so the change and its audit row
//! commit or roll back together.
//!
//! Secrets never reach a row; two layers, because each misses what the other catches: secret
//! values in innocuous fields are typed [`deskhaven_config::Secret`], and [`redact`] masks by
//! key name and value shape at any depth. `null` stays `null`: whether a secret is set is not
//! itself secret.

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

pub const MAX_LABEL_CHARS: usize = 256;

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

#[derive(Debug, Clone, PartialEq)]
pub struct NewConsoleAudit {
    pub actor: Actor,
    pub action: String,
    pub target_type: String,
    pub target_id: String,
    pub before: Option<Value>,
    pub after: Option<Value>,
    pub ip: Option<IpAddr>,
}

#[derive(Clone)]
pub struct ConsoleAuditLog {
    db: DatabaseConnection,
    clock: SharedClock,
}

impl ConsoleAuditLog {
    pub fn new(db: DatabaseConnection, clock: SharedClock) -> Self {
        Self { db, clock }
    }

    pub async fn record(&self, entry: &NewConsoleAudit) -> Result<i64, DbErr> {
        self.record_in(&self.db, entry).await
    }

    pub async fn record_in<C: ConnectionTrait>(
        &self,
        conn: &C,
        entry: &NewConsoleAudit,
    ) -> Result<i64, DbErr> {
        insert(conn, entry, self.clock.now_ms()).await
    }
}

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
