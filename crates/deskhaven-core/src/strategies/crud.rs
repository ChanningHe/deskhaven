//! Strategy CRUD. Creating changes no effective strategy (nothing refers to the new
//! strategy yet); updating and deleting do, and invalidate the cache after commit.
//!
//! The `*_guarded` variants are the console's: their [`TxGuard`] decides inside the write
//! transaction and its audit row commits with the change (see `console_audit`).

use deskhaven_db::begin_write;
use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::entity::strategy;
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DbErr, EntityTrait, ModelTrait, QueryFilter,
    QueryOrder, QuerySelect, SqlErr,
};

use super::options::{self, Options};
use super::{NewStrategy, Strategy, StrategyError, StrategyService, StrategyUpdate};
use crate::console_audit::TxGuard;
use crate::text::{self, NOTE_MAX_LEN};

const NAME_MAX_LEN: usize = 64;

impl StrategyService {
    pub async fn create(&self, new: NewStrategy) -> Result<Strategy, StrategyError> {
        self.create_guarded(new, TxGuard::none()).await
    }

    /// [`create`](Self::create) with `guard` deciding on the new strategy (no `before`).
    pub async fn create_guarded(
        &self,
        new: NewStrategy,
        guard: TxGuard<Strategy>,
    ) -> Result<Strategy, StrategyError> {
        let name = validate_name(&new.name)?;
        let note = validate_note(&new.note)?;
        let stored = validated_options(&new.options)?;
        self.detached(move |inner| async move {
            let now = inner.clock.now();
            let model = strategy::ActiveModel {
                name: Set(name),
                options: Set(stored),
                note: Set(note),
                managed_by: Set(ManagedBy::Ui),
                created_at: Set(now),
                updated_at: Set(now),
                ..Default::default()
            };
            let tx = begin_write(&inner.db).await?;
            let created = Strategy::from(model.insert(&tx).await.map_err(name_conflict)?);
            guard.check(&tx, None, Some(&created)).await?;
            tx.commit().await?;
            Ok(created)
        })
        .await
    }

    pub async fn get(&self, id: i64) -> Result<Strategy, StrategyError> {
        Ok(find(&self.inner.db, id).await?.into())
    }

    pub async fn get_by_name(&self, name: &str) -> Result<Strategy, StrategyError> {
        let found = strategy::Entity::find()
            .filter(strategy::Column::Name.eq(name.trim()))
            .one(&self.inner.db)
            .await?;
        Ok(found.ok_or(StrategyError::NotFound)?.into())
    }

    /// Every strategy, ordered by name.
    pub async fn list(&self) -> Result<Vec<Strategy>, StrategyError> {
        let rows = strategy::Entity::find()
            .order_by_asc(strategy::Column::Name)
            .all(&self.inner.db)
            .await?;
        Ok(rows.into_iter().map(Strategy::from).collect())
    }

    /// Applies `update`; any successful non-empty update advances the strategy version. An
    /// empty update changes nothing.
    pub async fn update(&self, id: i64, update: StrategyUpdate) -> Result<Strategy, StrategyError> {
        self.update_guarded(id, update, TxGuard::none()).await
    }

    /// [`update`](Self::update) with `guard` deciding on the strategy before and after.
    pub async fn update_guarded(
        &self,
        id: i64,
        update: StrategyUpdate,
        guard: TxGuard<Strategy>,
    ) -> Result<Strategy, StrategyError> {
        let name = update.name.as_deref().map(validate_name).transpose()?;
        let note = update.note.as_deref().map(validate_note).transpose()?;
        if let Some(replacement) = &update.options {
            options::validate(replacement)?;
        }
        self.detached(move |inner| async move {
            let tx = begin_write(&inner.db).await?;
            let current = writable(&tx, id).await?;
            let before = Strategy::from(current.clone());
            if update.is_empty() {
                return Ok(before);
            }
            let mut changes: strategy::ActiveModel = current.into();
            if update.options.is_some() || !update.options_patch.is_empty() {
                let base = update.options.unwrap_or_else(|| before.options.clone());
                let merged = options::patched(base, &update.options_patch);
                changes.options = Set(validated_options(&merged)?);
            }
            if let Some(name) = name {
                changes.name = Set(name);
            }
            if let Some(note) = note {
                changes.note = Set(note);
            }
            changes.updated_at = Set(inner.clock.now());
            let updated = Strategy::from(changes.update(&tx).await.map_err(name_conflict)?);
            guard.check(&tx, Some(&before), Some(&updated)).await?;
            tx.commit().await?;
            inner.cache.invalidate();
            Ok(updated)
        })
        .await
    }

    /// Deletes the strategy and, by cascade, its assignments.
    pub async fn delete(&self, id: i64) -> Result<(), StrategyError> {
        self.delete_guarded(id, TxGuard::none()).await
    }

    /// [`delete`](Self::delete) with `guard` deciding on the strategy (no `after`).
    pub async fn delete_guarded(
        &self,
        id: i64,
        guard: TxGuard<Strategy>,
    ) -> Result<(), StrategyError> {
        self.detached(move |inner| async move {
            let tx = begin_write(&inner.db).await?;
            let current = writable(&tx, id).await?;
            guard
                .check(&tx, Some(&Strategy::from(current.clone())), None)
                .await?;
            current.delete(&tx).await?;
            tx.commit().await?;
            inner.cache.invalidate();
            Ok(())
        })
        .await
    }
}

async fn find(db: &impl ConnectionTrait, id: i64) -> Result<strategy::Model, StrategyError> {
    let found = strategy::Entity::find_by_id(id).one(db).await?;
    found.ok_or(StrategyError::NotFound)
}

/// The strategy, locked for the rest of the transaction, if the console may change it.
async fn writable(db: &impl ConnectionTrait, id: i64) -> Result<strategy::Model, StrategyError> {
    let found = strategy::Entity::find_by_id(id)
        .lock_exclusive()
        .one(db)
        .await?;
    let strategy = found.ok_or(StrategyError::NotFound)?;
    match strategy.managed_by {
        ManagedBy::Config => Err(StrategyError::ManagedByConfig),
        _ => Ok(strategy),
    }
}

fn validated_options(options: &Options) -> Result<String, StrategyError> {
    options::validate(options)?;
    options::encode(options)
}

fn invalid(field: &'static str, reason: impl Into<String>) -> StrategyError {
    StrategyError::Invalid {
        field,
        reason: reason.into(),
    }
}

/// Trimmed name: 1-64 characters, no control characters.
fn validate_name(raw: &str) -> Result<String, StrategyError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(invalid("name", "must not be blank"));
    }
    match text::problem(name, NAME_MAX_LEN, false) {
        Some(reason) => Err(invalid("name", reason)),
        None => Ok(name.to_owned()),
    }
}

fn validate_note(raw: &str) -> Result<String, StrategyError> {
    match text::problem(raw, NOTE_MAX_LEN, true) {
        Some(reason) => Err(invalid("note", reason)),
        None => Ok(raw.to_owned()),
    }
}

fn name_conflict(err: DbErr) -> StrategyError {
    match err.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(_)) => StrategyError::Conflict { field: "name" },
        _ => StrategyError::Database(err),
    }
}
