//! Rows of `settings` / `settings_history`: decoding, locking, writing.
//!
//! Values are JSON text. A sensitive value is stored as the JSON string of
//! its `SecretBox` seal under [`value_context`]; history never holds a
//! secret or its ciphertext, only the `{"set": bool}` marker.

use deskhaven_config::settings::{Accepted, LoadWarning, Stored, lookup};
use deskhaven_db::entity::{setting, setting_history};
use deskhaven_db::time::TimestampMs;
use sea_orm::{
    ActiveModelTrait, ActiveValue::NotSet, ActiveValue::Set, ColumnTrait, ConnectionTrait,
    DatabaseTransaction, DbBackend, DbErr, EntityTrait, QueryFilter,
};
use serde_json::Value as Json;

use super::SettingsError;
use crate::crypto::SecretBox;

/// `SecretBox` context of a stored sensitive value (06-security §5): binds
/// the ciphertext to its key, so it cannot be moved to another setting.
pub fn value_context(key: &str) -> String {
    format!("settings.value:{key}")
}

/// Who and when, shared by every row of one batch.
pub(super) struct Stamp<'a> {
    pub(super) by: Option<i64>,
    pub(super) at: TimestampMs,
    pub(super) reason: Option<&'a str>,
}

pub(super) async fn fetch(conn: &impl ConnectionTrait) -> Result<Vec<setting::Model>, DbErr> {
    setting::Entity::find().all(conn).await
}

/// Serializes settings writers across processes. SQLite already holds the
/// write lock (`BEGIN IMMEDIATE`, bounded by its busy timeout); PostgreSQL
/// READ COMMITTED would otherwise allow write skew across keys of one
/// cross-key rule. Readers are not blocked. The wait is bounded so a stuck
/// transaction elsewhere surfaces as an error instead of a hang.
pub(super) async fn lock(tx: &DatabaseTransaction) -> Result<(), DbErr> {
    if tx.get_database_backend() == DbBackend::Postgres {
        tx.execute_unprepared(PG_LOCK_TIMEOUT).await?;
        tx.execute_unprepared("LOCK TABLE settings IN EXCLUSIVE MODE")
            .await?;
    }
    Ok(())
}

/// Matches the SQLite busy timeout (`deskhaven_db::SQLITE_BUSY_TIMEOUT`).
const PG_LOCK_TIMEOUT: &str = "SET LOCAL lock_timeout = '5s'";

/// Every row becomes a [`Stored`]; one that cannot be read keeps its
/// version (so it can be overwritten) and is reported as a warning.
pub(super) fn decode(
    rows: &[setting::Model],
    crypto: &SecretBox,
) -> (Vec<Stored>, Vec<LoadWarning>) {
    let mut warnings = Vec::new();
    let stored = rows
        .iter()
        .map(|row| {
            let value = decode_value(row, crypto)
                .map_err(|problem| {
                    warnings.push(LoadWarning {
                        key: row.key.clone(),
                        problem: format!("{problem}; using the default"),
                    })
                })
                .ok();
            Stored {
                key: row.key.clone(),
                value,
                version: row.version,
                by: row.updated_by,
                at: row.updated_at.timestamp_millis(),
            }
        })
        .collect();
    (stored, warnings)
}

fn decode_value(row: &setting::Model, crypto: &SecretBox) -> Result<Json, &'static str> {
    let value: Json =
        serde_json::from_str(&row.value).map_err(|_| "stored value is not valid JSON")?;
    match lookup(&row.key).is_some_and(|s| s.is_sensitive()) {
        true => open(crypto, &row.key, &value)
            .ok_or("stored secret cannot be decrypted (other master key, or moved/tampered)"),
        false => Ok(value),
    }
}

fn open(crypto: &SecretBox, key: &str, value: &Json) -> Option<Json> {
    let plaintext = crypto.open(&value_context(key), value.as_str()?).ok()?;
    let text = std::str::from_utf8(&plaintext).ok()?;
    Some(Json::String(text.to_owned()))
}

/// Writes one accepted change and its history row. `old` is the current row
/// (its version was already checked); `old_public` its public JSON form.
pub(super) async fn write(
    tx: &DatabaseTransaction,
    crypto: &SecretBox,
    accepted: &Accepted,
    old: Option<(&setting::Model, Json)>,
    stamp: &Stamp<'_>,
) -> Result<(), SettingsError> {
    let key = accepted.key();
    let version = old.as_ref().map_or(0, |(row, _)| row.version) + 1;
    let row = setting::ActiveModel {
        key: Set(key.to_owned()),
        value: Set(storage_text(crypto, accepted)?),
        version: Set(version),
        updated_by: Set(stamp.by),
        updated_at: Set(stamp.at),
    };
    match &old {
        Some((current, _)) => update_row(tx, row, key, current.version).await?,
        None => {
            row.insert(tx).await?;
        }
    }
    let old_value = old.map(|(row, public)| match accepted.setting().is_sensitive() {
        true => public.to_string(),
        false => row.value.clone(),
    });
    let history = setting_history::ActiveModel {
        id: NotSet,
        key: Set(key.to_owned()),
        old_value: Set(old_value),
        new_value: Set(accepted.value().to_json().to_string()),
        version: Set(version),
        updated_by: Set(stamp.by),
        updated_at: Set(stamp.at),
        reason: Set(stamp.reason.map(str::to_owned)),
    };
    history.insert(tx).await?;
    Ok(())
}

/// Compare-and-set on `version`: defense in depth behind [`lock`].
async fn update_row(
    tx: &DatabaseTransaction,
    row: setting::ActiveModel,
    key: &str,
    expected: i64,
) -> Result<(), SettingsError> {
    let result = setting::Entity::update(row)
        .validate()?
        .filter(setting::Column::Version.eq(expected))
        .exec_without_returning(tx)
        .await;
    match result {
        Ok(_) => Ok(()),
        Err(DbErr::RecordNotUpdated) => Err(SettingsError::VersionConflict {
            keys: vec![key.to_owned()],
        }),
        Err(err) => Err(err.into()),
    }
}

/// Secrets are always sealed; everything else is its public JSON form.
fn storage_text(crypto: &SecretBox, accepted: &Accepted) -> Result<String, SettingsError> {
    let json = match accepted.value().secret() {
        Some(secret) => {
            let sealed = crypto.seal(&value_context(accepted.key()), secret.expose().as_bytes())?;
            Json::String(sealed)
        }
        None => accepted.value().to_json(),
    };
    Ok(json.to_string())
}
