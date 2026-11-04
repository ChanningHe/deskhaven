//! Provider management (admin routes: M3-09). The client secret is sealed under
//! `oidc_providers.client_secret_enc:<id>` (06 §5); the provider is inserted first and the
//! secret written in the same transaction, once the id is known.
//!
//! Providers with `managed_by = config` (bootstrap file, 05 §4.1) are read-only here.
//! Deleting a provider with linked accounts is refused ([`ProviderError::InUse`]); its
//! in-flight flows go with it (cascade). Every write drops the login button cache and the
//! provider's discovery cache before returning. Each `*_guarded` variant runs a
//! [`TxGuard`] inside its write transaction (the console's audit row, the actor re-check).

mod buttons;
mod model;

use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::entity::oidc_provider::{self, Column};
use deskhaven_db::time::TimestampMs;
use deskhaven_db::{begin_write, is_foreign_key_violation};
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseTransaction, DbErr, EntityTrait,
    ModelTrait, QueryFilter, QueryOrder, QuerySelect, SqlErr,
};
use zeroize::Zeroizing;

pub(crate) use buttons::ButtonCache;
pub use model::{LoginButton, MAX_ICON_LEN, MAX_NAME_LEN, NewProvider, Provider, ProviderUpdate};

use super::OidcService;
use super::error::ProviderError;
use crate::console_audit::TxGuard;
use crate::crypto::CryptoError;

impl OidcService {
    pub async fn create_provider(&self, new: NewProvider) -> Result<Provider, ProviderError> {
        self.create_provider_guarded(new, TxGuard::none()).await
    }

    /// [`create_provider`](Self::create_provider), with `guard` deciding on the new
    /// provider (no `before`).
    pub async fn create_provider_guarded(
        &self,
        new: NewProvider,
        guard: TxGuard<Provider>,
    ) -> Result<Provider, ProviderError> {
        let name = model::name(&new.name)?;
        let now = self.inner.clock.now();
        let row = oidc_provider::ActiveModel {
            display_name: Set(model::display_name(&new.display_name, &name)?),
            name: Set(name),
            icon: Set(model::icon(new.icon.as_deref())?),
            issuer: Set(model::issuer(&new.issuer)?),
            client_id: Set(model::client_id(&new.client_id)?),
            client_secret_enc: Set(None),
            scopes: Set(model::scopes(new.scopes.as_deref())?),
            auto_provision: Set(new.auto_provision),
            group_claim: Set(model::group_claim(new.group_claim.as_deref())?),
            admin_group: Set(model::admin_group(new.admin_group.as_deref())?),
            enabled: Set(new.enabled),
            managed_by: Set(ManagedBy::Ui),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        let secret = model::client_secret(new.client_secret.as_deref())?;
        let tx = begin_write(&self.inner.db).await?;
        let created = row.insert(&tx).await.map_err(name_conflict)?;
        let created = Provider::from(match secret {
            Some(secret) => self.write_secret(&tx, created, Some(secret)).await?,
            None => created,
        });
        guard.check(&tx, None, Some(&created)).await?;
        tx.commit().await?;
        self.changed(created.id);
        Ok(created)
    }

    pub async fn get_provider(&self, id: i64) -> Result<Provider, ProviderError> {
        let found = oidc_provider::Entity::find_by_id(id)
            .one(&self.inner.db)
            .await?;
        found.map(Provider::from).ok_or(ProviderError::NotFound)
    }

    pub async fn get_provider_by_name(&self, name: &str) -> Result<Provider, ProviderError> {
        let found = self.find_by_name(&self.inner.db, name).await?;
        found.map(Provider::from).ok_or(ProviderError::NotFound)
    }

    /// Every provider, ordered by name.
    pub async fn list_providers(&self) -> Result<Vec<Provider>, ProviderError> {
        let rows = oidc_provider::Entity::find()
            .order_by_asc(Column::Name)
            .all(&self.inner.db)
            .await?;
        Ok(rows.into_iter().map(Provider::from).collect())
    }

    pub async fn update_provider(
        &self,
        id: i64,
        update: ProviderUpdate,
    ) -> Result<Provider, ProviderError> {
        self.update_provider_guarded(id, update, TxGuard::none())
            .await
    }

    /// [`update_provider`](Self::update_provider), with `guard` deciding on the provider
    /// before and after.
    pub async fn update_provider_guarded(
        &self,
        id: i64,
        update: ProviderUpdate,
        guard: TxGuard<Provider>,
    ) -> Result<Provider, ProviderError> {
        let tx = begin_write(&self.inner.db).await?;
        let current = writable(&tx, id).await?;
        let before = Provider::from(current.clone());
        let changes = changes(&current, &update, self.inner.clock.now())?;
        let mut updated = changes.update(&tx).await.map_err(name_conflict)?;
        if let Some(secret) = &update.client_secret {
            let secret = model::client_secret(secret.as_deref())?;
            updated = self.write_secret(&tx, updated, secret).await?;
        }
        let updated = Provider::from(updated);
        guard.check(&tx, Some(&before), Some(&updated)).await?;
        tx.commit().await?;
        self.changed(id);
        Ok(updated)
    }

    pub async fn delete_provider(&self, id: i64) -> Result<(), ProviderError> {
        self.delete_provider_guarded(id, TxGuard::none()).await
    }

    /// [`delete_provider`](Self::delete_provider), with `guard` deciding on the provider
    /// (no `after`).
    pub async fn delete_provider_guarded(
        &self,
        id: i64,
        guard: TxGuard<Provider>,
    ) -> Result<(), ProviderError> {
        let tx = begin_write(&self.inner.db).await?;
        let current = writable(&tx, id).await?;
        let before = Provider::from(current.clone());
        guard.check(&tx, Some(&before), None).await?;
        current.delete(&tx).await.map_err(|err| {
            if is_foreign_key_violation(&err) {
                ProviderError::InUse
            } else {
                ProviderError::Database(err)
            }
        })?;
        tx.commit().await?;
        self.changed(id);
        Ok(())
    }

    /// The enabled provider the client names as `op`.
    pub(crate) async fn enabled_provider(
        &self,
        name: &str,
    ) -> Result<Option<oidc_provider::Model>, DbErr> {
        let found = self.find_by_name(&self.inner.db, name).await?;
        Ok(found.filter(|p| p.enabled))
    }

    /// The provider's client secret, opened; `None` for a public client.
    pub(crate) fn client_secret(
        &self,
        provider: &oidc_provider::Model,
    ) -> Result<Option<Zeroizing<String>>, CryptoError> {
        let Some(sealed) = provider.client_secret_enc.as_deref() else {
            return Ok(None);
        };
        let plain = self
            .inner
            .crypto
            .open(&secret_context(provider.id), sealed)?;
        let text = String::from_utf8(plain.to_vec()).map_err(|_| CryptoError::Malformed)?;
        Ok(Some(Zeroizing::new(text)))
    }

    async fn find_by_name<C: ConnectionTrait>(
        &self,
        conn: &C,
        name: &str,
    ) -> Result<Option<oidc_provider::Model>, DbErr> {
        oidc_provider::Entity::find()
            .filter(Column::Name.eq(name))
            .one(conn)
            .await
    }

    async fn write_secret(
        &self,
        tx: &DatabaseTransaction,
        row: oidc_provider::Model,
        secret: Option<String>,
    ) -> Result<oidc_provider::Model, ProviderError> {
        let secret = secret.map(Zeroizing::new);
        let sealed = match &secret {
            Some(plain) => Some(
                self.inner
                    .crypto
                    .seal(&secret_context(row.id), plain.as_bytes())
                    .map_err(ProviderError::Crypto)?,
            ),
            None => None,
        };
        let mut active: oidc_provider::ActiveModel = row.into();
        active.client_secret_enc = Set(sealed);
        Ok(active.update(tx).await?)
    }

    /// After a committed write: forget what was derived from the old row.
    fn changed(&self, id: i64) {
        self.inner.buttons.invalidate();
        self.inner.discovery.forget(id);
    }
}

/// Sealing context of a provider's client secret.
fn secret_context(id: i64) -> String {
    format!("oidc_providers.client_secret_enc:{id}")
}

/// The row, locked for the transaction; refused when the configuration owns it.
async fn writable(
    tx: &DatabaseTransaction,
    id: i64,
) -> Result<oidc_provider::Model, ProviderError> {
    let found = oidc_provider::Entity::find_by_id(id)
        .lock_exclusive()
        .one(tx)
        .await?;
    let row = found.ok_or(ProviderError::NotFound)?;
    if row.managed_by == ManagedBy::Config {
        return Err(ProviderError::ManagedByConfig);
    }
    Ok(row)
}

/// The validated changes of `update` (the secret is written separately).
fn changes(
    current: &oidc_provider::Model,
    update: &ProviderUpdate,
    now: TimestampMs,
) -> Result<oidc_provider::ActiveModel, ProviderError> {
    let mut row: oidc_provider::ActiveModel = current.clone().into();
    let name = match &update.name {
        Some(raw) => model::name(raw)?,
        None => current.name.clone(),
    };
    if let Some(raw) = &update.display_name {
        row.display_name = Set(model::display_name(raw, &name)?);
    }
    row.name = Set(name);
    if let Some(icon) = &update.icon {
        row.icon = Set(model::icon(icon.as_deref())?);
    }
    if let Some(raw) = &update.client_id {
        row.client_id = Set(model::client_id(raw)?);
    }
    if let Some(raw) = &update.scopes {
        row.scopes = Set(model::scopes(Some(raw))?);
    }
    if let Some(on) = update.auto_provision {
        row.auto_provision = Set(on);
    }
    if let Some(claim) = &update.group_claim {
        row.group_claim = Set(model::group_claim(claim.as_deref())?);
    }
    if let Some(group) = &update.admin_group {
        row.admin_group = Set(model::admin_group(group.as_deref())?);
    }
    if let Some(on) = update.enabled {
        row.enabled = Set(on);
    }
    row.updated_at = Set(now);
    Ok(row)
}

fn name_conflict(err: DbErr) -> ProviderError {
    match err.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(_)) => ProviderError::Conflict,
        _ => ProviderError::Database(err),
    }
}
