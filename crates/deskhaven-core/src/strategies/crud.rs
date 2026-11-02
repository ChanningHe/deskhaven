//! Strategy CRUD. Creating changes no effective strategy (nothing refers to the new
//! strategy yet); updating and deleting do, and invalidate the cache after commit.

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
use crate::text::{self, NOTE_MAX_LEN};

const NAME_MAX_LEN: usize = 64;

impl StrategyService {
    pub async fn create(&self, new: NewStrategy) -> Result<Strategy, StrategyError> {
        let name = validate_name(&new.name)?;
        let note = validate_note(&new.note)?;
        let stored = validated_options(&new.options)?;
        let now = self.inner.clock.now();
        let model = strategy::ActiveModel {
            name: Set(name),
            options: Set(stored),
            note: Set(note),
            managed_by: Set(ManagedBy::Ui),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        let created = model.insert(&self.inner.db).await.map_err(name_conflict)?;
        created.try_into()
    }

    pub async fn get(&self, id: i64) -> Result<Strategy, StrategyError> {
        find(&self.inner.db, id).await?.try_into()
    }

    pub async fn get_by_name(&self, name: &str) -> Result<Strategy, StrategyError> {
        let found = strategy::Entity::find()
            .filter(strategy::Column::Name.eq(name.trim()))
            .one(&self.inner.db)
            .await?;
        found.ok_or(StrategyError::NotFound)?.try_into()
    }

    /// Every strategy, ordered by name.
    pub async fn list(&self) -> Result<Vec<Strategy>, StrategyError> {
        let rows = strategy::Entity::find()
            .order_by_asc(strategy::Column::Name)
            .all(&self.inner.db)
            .await?;
        rows.into_iter().map(Strategy::try_from).collect()
    }

    /// Applies `update`; any successful update advances the strategy version.
    pub async fn update(&self, id: i64, update: StrategyUpdate) -> Result<Strategy, StrategyError> {
        let name = update.name.as_deref().map(validate_name).transpose()?;
        let note = update.note.as_deref().map(validate_note).transpose()?;
        let stored = update.options.as_ref().map(validated_options).transpose()?;
        self.detached(move |inner| async move {
            let tx = begin_write(&inner.db).await?;
            let current = writable(&tx, id).await?;
            let mut changes: strategy::ActiveModel = current.into();
            if let Some(name) = name {
                changes.name = Set(name);
            }
            if let Some(stored) = stored {
                changes.options = Set(stored);
            }
            if let Some(note) = note {
                changes.note = Set(note);
            }
            changes.updated_at = Set(inner.clock.now());
            let updated = changes.update(&tx).await.map_err(name_conflict)?;
            tx.commit().await?;
            inner.cache.invalidate();
            Strategy::try_from(updated)
        })
        .await
    }

    /// Deletes the strategy and, by cascade, its assignments.
    pub async fn delete(&self, id: i64) -> Result<(), StrategyError> {
        self.detached(move |inner| async move {
            let tx = begin_write(&inner.db).await?;
            let current = writable(&tx, id).await?;
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
