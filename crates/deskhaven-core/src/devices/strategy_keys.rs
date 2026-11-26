//! The client keeps every option it was sent until it gets that key again, and an empty value
//! deletes it: a key that disappears from the effective strategy must be sent once more as `""`.
//!
//! Invariant: the stored set is a superset of the keys the device may hold. It grows before a
//! send and shrinks only when the device confirms a strategy (the client stores `modified_at`
//! only after applying it). Both writes are compare-and-set on the stored text, so a lost race
//! only leaves a larger set.
//!
//! Keyed by RustDesk id, not by row: deleting a device does not reset its client, which still
//! holds what it was sent.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Mutex, MutexGuard, PoisonError};

use deskhaven_db::entity::device_strategy_key::{self, Entity as StoredKeys};
use deskhaven_db::time;
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter,
    TryInsertResult,
};

use crate::strategies::{EffectiveStrategy, Options};

/// Beyond that the table is cleared (safe: the stored set stays a superset).
pub const MAX_PENDING: usize = 100_000;
/// Concurrent heartbeats of one device; the per-device interval makes even one retry rare.
const GROW_ATTEMPTS: usize = 3;

type Keys = BTreeSet<String>;

#[derive(Debug, thiserror::Error)]
pub(crate) enum KeysError {
    #[error("the stored strategy keys kept changing")]
    Contended,
    #[error("strategy key storage failed")]
    Database(#[from] DbErr),
}

#[derive(Clone, Debug)]
struct Pending {
    modified_at: i64,
    live: Keys,
    stored: Option<String>,
}

pub(crate) struct StrategyDeliveries {
    db: DatabaseConnection,
    capacity: usize,
    pending: Mutex<HashMap<String, Pending>>,
}

impl StrategyDeliveries {
    pub fn new(db: DatabaseConnection) -> Self {
        Self::with_capacity(db, MAX_PENDING)
    }

    pub fn with_capacity(db: DatabaseConnection, capacity: usize) -> Self {
        Self {
            db,
            capacity,
            pending: Mutex::default(),
        }
    }

    #[cfg(test)]
    fn pending_len(&self) -> usize {
        self.lock().len()
    }

    /// On error nothing may be sent.
    pub async fn prepare(
        &self,
        rustdesk_id: &str,
        effective: &EffectiveStrategy,
    ) -> Result<Options, KeysError> {
        let live: Keys = effective
            .live_options()
            .map(|(k, _)| k.to_owned())
            .collect();
        let (stored_keys, stored) = self.grow(rustdesk_id, &live).await?;
        let mut options = effective.options.clone();
        for key in stored_keys {
            options.entry(key).or_default();
        }
        let pending = Pending {
            modified_at: effective.modified_at,
            live,
            stored,
        };
        let mut table = self.lock();
        if table.len() >= self.capacity && !table.contains_key(rustdesk_id) {
            table.clear();
        }
        table.insert(rustdesk_id.to_owned(), pending);
        Ok(options)
    }

    pub async fn confirm(&self, rustdesk_id: &str, modified_at: i64) -> Result<(), DbErr> {
        let confirmed = self
            .lock()
            .get(rustdesk_id)
            .filter(|p| p.modified_at == modified_at)
            .cloned();
        let Some(pending) = confirmed else {
            return Ok(());
        };
        // No row means nothing was sent with a value: nothing to shrink.
        let live_text = encode(&pending.live);
        if let Some(stored) = &pending.stored
            && *stored != live_text
        {
            // A failed swap means the row changed meanwhile: it stays a superset.
            self.swap(rustdesk_id, stored, &live_text).await?;
        }
        let mut table = self.lock();
        if table
            .get(rustdesk_id)
            .is_some_and(|p| p.modified_at == modified_at)
        {
            table.remove(rustdesk_id);
        }
        Ok(())
    }

    async fn grow(
        &self,
        rustdesk_id: &str,
        live: &Keys,
    ) -> Result<(Keys, Option<String>), KeysError> {
        for _ in 0..GROW_ATTEMPTS {
            let row = StoredKeys::find_by_id(rustdesk_id.to_owned())
                .one(&self.db)
                .await?;
            let text = row.map(|r| r.option_keys);
            let previous = text.as_deref().map(decode).unwrap_or_default();
            let grown: Keys = previous.union(live).cloned().collect();
            if grown == previous {
                return Ok((grown, text));
            }
            let grown_text = encode(&grown);
            let written = match &text {
                Some(old) => self.swap(rustdesk_id, old, &grown_text).await?,
                None => self.insert(rustdesk_id, &grown_text).await?,
            };
            if written {
                return Ok((grown, Some(grown_text)));
            }
        }
        Err(KeysError::Contended)
    }

    async fn swap(&self, rustdesk_id: &str, old: &str, new: &str) -> Result<bool, DbErr> {
        let updated = StoredKeys::update_many()
            .col_expr(device_strategy_key::Column::OptionKeys, Expr::value(new))
            .col_expr(
                device_strategy_key::Column::UpdatedAt,
                Expr::value(time::now()),
            )
            .filter(device_strategy_key::Column::RustdeskId.eq(rustdesk_id))
            .filter(device_strategy_key::Column::OptionKeys.eq(old))
            .exec(&self.db)
            .await?;
        Ok(updated.rows_affected == 1)
    }

    async fn insert(&self, rustdesk_id: &str, keys: &str) -> Result<bool, DbErr> {
        let row = device_strategy_key::ActiveModel {
            rustdesk_id: Set(rustdesk_id.to_owned()),
            option_keys: Set(keys.to_owned()),
            updated_at: Set(time::now()),
        };
        let res = StoredKeys::insert(row)
            .on_conflict_do_nothing_on([device_strategy_key::Column::RustdeskId])
            .exec_without_returning(&self.db)
            .await?;
        Ok(matches!(res, TryInsertResult::Inserted(n) if n > 0))
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Pending>> {
        // Map operations cannot leave it inconsistent.
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// So the expiry job's grace period runs from the deletion.
pub(crate) async fn stamp_deleted<C: ConnectionTrait>(
    conn: &C,
    rustdesk_id: &str,
) -> Result<(), DbErr> {
    StoredKeys::update_many()
        .col_expr(
            device_strategy_key::Column::UpdatedAt,
            Expr::value(time::now()),
        )
        .filter(device_strategy_key::Column::RustdeskId.eq(rustdesk_id))
        .exec(conn)
        .await?;
    Ok(())
}

/// One text per set, so compare-and-set can compare text.
fn encode(keys: &Keys) -> String {
    let names: Vec<&str> = keys.iter().map(String::as_str).collect();
    serde_json::Value::from(names).to_string()
}

/// An unreadable value counts as empty; it still matches its own text in compare-and-set.
fn decode(text: &str) -> Keys {
    serde_json::from_str(text).unwrap_or_else(|_| {
        tracing::warn!("unreadable device_strategy_keys row; treated as empty");
        Keys::new()
    })
}

#[cfg(test)]
mod tests;
