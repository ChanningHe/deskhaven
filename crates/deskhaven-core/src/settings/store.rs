//! Rows of `settings` / `settings_history`: decoding, locking, writing.
//!
//! Values are JSON text. A sensitive value is stored as the JSON string of
//! its `SecretBox` seal under [`value_context`]; history never holds a
//! secret or its ciphertext, only the `{"set": bool}` marker.

use std::collections::BTreeMap;

use deskhaven_config::settings::{Accepted, LoadWarning, Stored, lookup};
use deskhaven_db::entity::{setting, setting_history};
use deskhaven_db::time::TimestampMs;
use sea_orm::{
    ActiveModelTrait, ActiveValue::NotSet, ActiveValue::Set, ColumnTrait, ConnectionTrait,
    DatabaseTransaction, DbBackend, DbErr, EntityTrait, QueryFilter, QuerySelect, SqlErr,
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
        tx.execute_unprepared(PG_STATEMENT_TIMEOUT).await?;
        tx.execute_unprepared("LOCK TABLE settings IN EXCLUSIVE MODE")
            .await?;
    }
    Ok(())
}

/// Matches the SQLite busy timeout (`deskhaven_db::SQLITE_BUSY_TIMEOUT`).
const PG_LOCK_TIMEOUT: &str = "SET LOCAL lock_timeout = '5s'";
/// Per statement; the whole batch is bounded by `TX_TIMEOUT` in `mod.rs`.
const PG_STATEMENT_TIMEOUT: &str = "SET LOCAL statement_timeout = '10s'";

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
/// (its version was already checked) with the public JSON text of its
/// previous effective value (`null` when the row was unreadable), which is
/// what the history records. `version` is the one the row gets (the history
/// entry of a reset gets it too: the row is gone, the version is not reused).
///
/// Lock order note: once `updated_by` references `users(id)`, these writes
/// take a key-share lock on the user row after the settings table lock; any
/// path that deletes users and then writes settings must not hold user row
/// locks first, or it can deadlock with a settings update.
pub(super) async fn write(
    tx: &DatabaseTransaction,
    crypto: &SecretBox,
    accepted: &Accepted,
    old: Option<(&setting::Model, String)>,
    version: i64,
    stamp: &Stamp<'_>,
) -> Result<(), SettingsError> {
    let key = accepted.key();
    match (&old, accepted.is_reset()) {
        (Some((current, _)), true) => delete_row(tx, key, current.version).await?,
        // Nothing stored, nothing to drop (the service filters these out).
        (None, true) => return Ok(()),
        (existing, false) => {
            let row = setting::ActiveModel {
                key: Set(key.to_owned()),
                value: Set(storage_text(crypto, accepted)?),
                version: Set(version),
                updated_by: Set(stamp.by),
                updated_at: Set(stamp.at),
            };
            match existing {
                Some((current, _)) => update_row(tx, row, key, current.version).await?,
                None => insert_row(tx, row, key).await?,
            }
        }
    }
    let history = setting_history::ActiveModel {
        id: NotSet,
        key: Set(key.to_owned()),
        old_value: Set(old.map(|(_, previous)| previous)),
        new_value: Set(accepted.value().to_json().to_string()),
        version: Set(version),
        updated_by: Set(stamp.by),
        updated_at: Set(stamp.at),
        reason: Set(stamp.reason.map(str::to_owned)),
        reset: Set(accepted.is_reset()),
    };
    history.insert(tx).await?;
    Ok(())
}

/// The highest history version of each of `keys` that has one. A key without a
/// row continues after it, so no version is ever reused.
pub(super) async fn version_floors<'a>(
    conn: &impl ConnectionTrait,
    keys: impl IntoIterator<Item = &'a str>,
) -> Result<BTreeMap<String, i64>, DbErr> {
    let keys: Vec<&str> = keys.into_iter().collect();
    if keys.is_empty() {
        return Ok(BTreeMap::new());
    }
    let rows = setting_history::Entity::find()
        .select_only()
        .column(setting_history::Column::Key)
        .column_as(setting_history::Column::Version.max(), "floor")
        .filter(setting_history::Column::Key.is_in(keys))
        .group_by(setting_history::Column::Key)
        .into_tuple::<(String, i64)>()
        .all(conn)
        .await?;
    Ok(rows.into_iter().collect())
}

/// Compare-and-set deletion on `version`: a reset of a row that changed meanwhile
/// is a conflict.
async fn delete_row(
    tx: &DatabaseTransaction,
    key: &str,
    expected: i64,
) -> Result<(), SettingsError> {
    let result = setting::Entity::delete_many()
        .filter(setting::Column::Key.eq(key))
        .filter(setting::Column::Version.eq(expected))
        .exec(tx)
        .await?;
    match result.rows_affected {
        0 => Err(SettingsError::VersionConflict {
            keys: vec![key.to_owned()],
        }),
        _ => Ok(()),
    }
}

/// A row that appeared since the locked read (only possible if the lock
/// is bypassed) is a version conflict, not a storage failure.
async fn insert_row(
    tx: &DatabaseTransaction,
    row: setting::ActiveModel,
    key: &str,
) -> Result<(), SettingsError> {
    match row.insert(tx).await {
        Ok(_) => Ok(()),
        Err(err) if matches!(err.sql_err(), Some(SqlErr::UniqueConstraintViolation(_))) => {
            Err(SettingsError::VersionConflict {
                keys: vec![key.to_owned()],
            })
        }
        Err(err) => Err(err.into()),
    }
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use deskhaven_config::settings::{Change, PinSet, prepare};
    use deskhaven_db::testing::{TestDb, each_migrated_backend};
    use deskhaven_db::{begin_write, time};

    use super::*;
    use crate::crypto::MasterKey;

    #[tokio::test]
    async fn insert_racing_an_existing_row_is_a_version_conflict() {
        each_migrated_backend(async |db: &TestDb| {
            let change = Change {
                key: "smtp.port".to_owned(),
                value: serde_json::json!(2525),
                version: 0,
            };
            let accepted = prepare(&PinSet::empty(), &[change]).unwrap();
            let crypto = SecretBox::new(&MasterKey::from_bytes(&[7; 32]));
            let stamp = Stamp {
                by: None,
                at: time::now(),
                reason: None,
            };
            let tx = begin_write(&db.conn).await.unwrap();
            write(&tx, &crypto, &accepted[0], None, 1, &stamp)
                .await
                .unwrap();

            // As if the locked read had missed the row just inserted.
            let err = write(&tx, &crypto, &accepted[0], None, 1, &stamp)
                .await
                .unwrap_err();

            assert!(
                matches!(&err, SettingsError::VersionConflict { keys } if keys == &["smtp.port"]),
                "{}: {err:?}",
                db.backend
            );
        })
        .await;
    }
}
