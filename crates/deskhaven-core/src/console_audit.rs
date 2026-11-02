//! Console audit log (04 §7 `console_audit`, 06 §7): who changed what through the console,
//! an API token or the CLI. Append-only from here; retention is the background jobs'
//! business.
//!
//! Secrets never reach a row. Two layers, because each misses what the other catches:
//! - **Type**: secret values that sit in innocuously named fields (a setting's `value`, a
//!   generic map entry) are [`deskhaven_config::Secret`]s in the snapshot the caller
//!   serializes; they render as [`REDACTED`] (or `""` when unset).
//! - **Name**: [`redact`] masks every object member whose key contains one of
//!   [`SECRET_KEY_FRAGMENTS`] (ASCII case-insensitive), at any depth, so a plain `String`
//!   password field a caller forgot to wrap is still masked. It errs on the side of masking:
//!   a non-secret key that happens to match (e.g. `api_token_id`) loses its value too.
//!
//! `null` stays `null` under either layer: whether a secret is set is not itself secret.

use std::net::IpAddr;

use deskhaven_config::REDACTED;
use deskhaven_db::entity::console_audit;
use deskhaven_db::time;
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, DatabaseConnection, DbErr};
use serde_json::{Map, Value};

pub use deskhaven_db::entity::console_audit::ActorKind;

use crate::clock::SharedClock;

/// Object keys whose values [`redact`] masks: a key matches when it contains a fragment,
/// ignoring ASCII case.
pub const SECRET_KEY_FRAGMENTS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "private_key",
    "recovery_code",
    "credential",
];

/// Longest stored `action`, `target_type` and `target_id` (characters; longer is cut).
pub const MAX_LABEL_CHARS: usize = 256;

/// Who performed an audited action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    /// Inserts one row stamped with the clock's time; returns its id.
    pub async fn record(&self, entry: &NewConsoleAudit) -> Result<i64, DbErr> {
        let now = self.clock.now_ms();
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
        row.insert(&self.db).await.map(|row| row.id)
    }
}

/// `value` with every member whose key names a secret (see [`SECRET_KEY_FRAGMENTS`])
/// replaced by [`REDACTED`], at any depth. `null` members stay `null`.
pub fn redact(value: &Value) -> Value {
    match value {
        Value::Object(members) => Value::Object(
            members
                .iter()
                .map(|(key, member)| (key.clone(), redact_member(key, member)))
                .collect::<Map<_, _>>(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redact).collect()),
        scalar => scalar.clone(),
    }
}

fn redact_member(key: &str, member: &Value) -> Value {
    if member.is_null() || !is_secret_key(key) {
        return redact(member);
    }
    Value::String(REDACTED.to_owned())
}

fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    SECRET_KEY_FRAGMENTS.iter().any(|f| key.contains(f))
}

fn label(s: &str) -> String {
    s.chars().take(MAX_LABEL_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use deskhaven_config::Secret;
    use serde_json::json;

    use super::*;

    #[test]
    fn masks_secret_keys_at_any_depth_and_keeps_the_rest() {
        let input = json!({
            "username": "alice",
            "Password": "hunter2",
            "smtp": { "bind_passwd": 1, "host": "mail" },
            "providers": [{ "client_secret": "s3", "name": "idp" }],
            "api_token": { "nested": "whole object" },
            "totp_secret": null,
        });

        let got = redact(&input);

        assert_eq!(
            got,
            json!({
                "username": "alice",
                "Password": REDACTED,
                "smtp": { "bind_passwd": REDACTED, "host": "mail" },
                "providers": [{ "client_secret": REDACTED, "name": "idp" }],
                "api_token": REDACTED,
                "totp_secret": null,
            })
        );
    }

    #[test]
    fn secret_wrapper_masks_innocuous_field_names() {
        #[derive(serde::Serialize)]
        struct SettingView {
            key: &'static str,
            value: Secret,
        }
        let view = SettingView {
            key: "smtp.password",
            value: Secret::new("hunter2"),
        };

        let got = redact(&serde_json::to_value(view).unwrap());

        assert_eq!(got, json!({ "key": "smtp.password", "value": REDACTED }));
    }

    #[test]
    fn scalars_and_arrays_without_objects_are_unchanged() {
        for value in [
            json!(1),
            json!("password"),
            json!([1, "token"]),
            json!(null),
        ] {
            assert_eq!(redact(&value), value);
        }
    }

    #[test]
    fn labels_are_bounded() {
        let long = "é".repeat(MAX_LABEL_CHARS + 5);

        assert_eq!(label(&long).chars().count(), MAX_LABEL_CHARS);
        assert_eq!(label("user"), "user");
    }
}
