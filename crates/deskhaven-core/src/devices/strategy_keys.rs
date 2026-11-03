//! Which strategy options a device may still hold (M2-14 handover from M2-13).
//!
//! The client only touches the options it receives: a key sent with a value stays on the
//! device until the server sends that key again, and an empty value deletes it
//! (`src/hbbs_http/sync.rs:295-311`). A key that disappears from the effective strategy
//! (option removed, strategy or assignment removed, device moved to another group or owner)
//! must therefore be sent once more as `""`. To know which keys those are, the server keeps,
//! per device, the keys it sent with a non-empty value: `device_strategy_keys`.
//!
//! **Invariant**: the stored set is a superset of the keys the device may hold.
//! - Before a strategy is sent, the stored set grows by the keys sent with a value
//!   ([`StrategyDeliveries::prepare`]); if that write fails nothing is sent.
//! - It shrinks to exactly the keys of one sent strategy only when the device confirms that
//!   strategy by reporting its `modified_at` ([`StrategyDeliveries::confirm`]); the client
//!   stores `modified_at` only after applying the options (`sync.rs:261-275`), so a lost
//!   reply or a rejected update never shrinks it.
//!
//! Both writes are compare-and-set statements on the stored text, so no transaction or lock
//! is needed and a lost race only leaves a larger set (an extra `""` later, never a missed
//! one). Sending `""` for a key the device does not hold is harmless.
//!
//! **Keyed by RustDesk id**, not by `devices` row: deleting a device does not reset its
//! client, which registers again as a new row and still holds what it was sent. Rows are
//! therefore never deleted with the device (an extra `""` for a key the client lacks is
//! harmless).
//!
//! **Restart**: the stored set survives, so a device that was offline while a key was
//! removed, across any number of restarts, still gets its `""`. What is lost is the pending
//! confirmation (memory, [`MAX_PENDING`] devices at most): the stored set then stays larger
//! than needed until the next strategy the device confirms, i.e. at worst a few redundant
//! `""`. Restoring an older database can forget keys sent after the backup; nothing can
//! recover those.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Mutex, MutexGuard, PoisonError};

use deskhaven_db::entity::device_strategy_key::{self, Entity as StoredKeys};
use deskhaven_db::time;
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, TryInsertResult};

use crate::strategies::{EffectiveStrategy, Options};

/// Devices whose confirmation is awaited, at most; beyond that the table is cleared (safe,
/// see the module documentation).
pub const MAX_PENDING: usize = 100_000;
/// Compare-and-set attempts before giving up on growing the stored set (concurrent
/// heartbeats of one device; the per-device interval makes even one retry rare).
const GROW_ATTEMPTS: usize = 3;

type Keys = BTreeSet<String>;

/// Why a strategy could not be prepared.
#[derive(Debug, thiserror::Error)]
pub(crate) enum KeysError {
    #[error("the stored strategy keys kept changing")]
    Contended,
    #[error("strategy key storage failed")]
    Database(#[from] DbErr),
}

/// A strategy sent and not yet confirmed.
#[derive(Clone, Debug)]
struct Pending {
    modified_at: i64,
    /// Keys sent with a value: what the stored set shrinks to on confirmation.
    live: Keys,
    /// The stored text right after the send (`None`: no row).
    stored: Option<String>,
}

/// The stored key sets and the pending confirmations.
pub(crate) struct StrategyDeliveries {
    db: DatabaseConnection,
    /// Pending confirmations at most ([`MAX_PENDING`]).
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

    /// The `config_options` to send device `rustdesk_id` for `effective`: its options (empty
    /// values included) plus `""` for every stored key it no longer has. Grows the stored set
    /// first; on error nothing may be sent.
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

    /// Device `rustdesk_id` reports `modified_at`, the server's current value: if that is the
    /// strategy it was last sent, the stored set shrinks to that strategy's keys. Memory only
    /// unless such a shrink is due (once per sent strategy).
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

    /// Adds `live` to the stored set; returns the set and its stored text.
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

    /// `true` if the row of `rustdesk_id` held `old` and now holds `new`.
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

    /// `true` if the row was created (`false`: one appeared meanwhile).
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

/// Sorted JSON array: one text per set, so compare-and-set can compare text.
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
