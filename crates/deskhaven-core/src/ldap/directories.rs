//! Directory management (admin routes: M3-09). The bind password is sealed under
//! `ldap_directories.bind_password_enc:<id>` (06 §5); a new row is inserted first and the
//! password written in the same transaction, once the id is known. Rows with
//! `managed_by = config` (05 §4.1) are read-only here. P0 allows one enabled directory
//! (unique partial index): a second is a [`DirectoryError::Conflict`] on `enabled`. An
//! update that changes the URL, StartTLS, certificate verification or the bind DN must
//! carry the bind password again (else `Invalid` on `bind_password`), so console access
//! alone cannot redirect the stored password to another server.

use deskhaven_db::begin_write;
use deskhaven_db::entity::ldap_directory::{self, Column};
use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::time::TimestampMs;
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseTransaction, DbErr, EntityTrait, ModelTrait,
    QueryFilter, QueryOrder, QuerySelect, SqlErr,
};
use zeroize::Zeroizing;

use super::error::DirectoryError;
use super::model::{Directory, DirectoryUpdate, Fields, NewDirectory};
use super::settings::DirectorySettings;
use super::{LdapService, validate};

impl LdapService {
    pub async fn create_directory(&self, new: NewDirectory) -> Result<Directory, DirectoryError> {
        let fields = Fields::of_new(&new)?;
        let password = validate::bind_password(new.bind_password.as_deref())?;
        validate::bind_pair(fields.bind_dn.is_some(), password.is_some())?;
        let password = password.map(Zeroizing::new);
        let now = self.inner.clock.now();
        let mut row = ldap_directory::ActiveModel {
            managed_by: Set(ManagedBy::Ui),
            created_at: Set(now),
            bind_password_enc: Set(None),
            ..Default::default()
        };
        apply(&mut row, fields, now);
        let tx = begin_write(&self.inner.db).await?;
        let created = row.insert(&tx).await.map_err(conflict)?;
        let created = match password {
            Some(password) => self.write_password(&tx, created, Some(&password)).await?,
            None => created,
        };
        tx.commit().await?;
        Ok(created.into())
    }

    pub async fn get_directory(&self, id: i64) -> Result<Directory, DirectoryError> {
        let found = ldap_directory::Entity::find_by_id(id)
            .one(&self.inner.db)
            .await?;
        found.map(Directory::from).ok_or(DirectoryError::NotFound)
    }

    /// Every directory, ordered by name.
    pub async fn list_directories(&self) -> Result<Vec<Directory>, DirectoryError> {
        let rows = ldap_directory::Entity::find()
            .order_by_asc(Column::Name)
            .all(&self.inner.db)
            .await?;
        Ok(rows.into_iter().map(Directory::from).collect())
    }

    pub async fn update_directory(
        &self,
        id: i64,
        update: DirectoryUpdate,
    ) -> Result<Directory, DirectoryError> {
        let tx = begin_write(&self.inner.db).await?;
        let current = writable(&tx, id).await?;
        let fields = Fields::updated(&current, &update)?;
        // The stored password must not follow a changed destination (or lose its TLS)
        // without whoever changes it knowing it: it could be sent to their server.
        let moved = fields.url != current.url
            || fields.start_tls != current.start_tls
            || fields.tls_verify != current.tls_verify
            || fields.bind_dn != current.bind_dn;
        if moved && fields.bind_dn.is_some() && update.bind_password.is_none() {
            let reason = "re-enter when the connection changes";
            return Err(validate::invalid("bind_password", reason));
        }
        let password = match &update.bind_password {
            Some(raw) => Some(validate::bind_password(raw.as_deref())?.map(Zeroizing::new)),
            None => None,
        };
        let has_password = match &password {
            Some(new) => new.is_some(),
            None => current.bind_password_enc.is_some(),
        };
        validate::bind_pair(fields.bind_dn.is_some(), has_password)?;
        let mut row: ldap_directory::ActiveModel = current.into();
        apply(&mut row, fields, self.inner.clock.now());
        let mut updated = row.update(&tx).await.map_err(conflict)?;
        if let Some(password) = password {
            updated = self.write_password(&tx, updated, password.as_ref()).await?;
        }
        tx.commit().await?;
        Ok(updated.into())
    }

    pub async fn delete_directory(&self, id: i64) -> Result<(), DirectoryError> {
        let tx = begin_write(&self.inner.db).await?;
        let current = writable(&tx, id).await?;
        current.delete(&tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// The stored directory with its bind password opened, for diagnostics.
    pub async fn stored_settings(&self, id: i64) -> Result<DirectorySettings, DirectoryError> {
        let found = ldap_directory::Entity::find_by_id(id)
            .one(&self.inner.db)
            .await?;
        let row = found.ok_or(DirectoryError::NotFound)?;
        self.settings_of(&row)
    }

    /// The enabled directory, if any, with its bind password opened.
    pub(crate) async fn enabled_settings(
        &self,
    ) -> Result<Option<DirectorySettings>, DirectoryError> {
        let found = ldap_directory::Entity::find()
            .filter(Column::Enabled.eq(true))
            .one(&self.inner.db)
            .await?;
        found.map(|row| self.settings_of(&row)).transpose()
    }

    /// Whether a directory is enabled (no secret is opened).
    pub(crate) async fn has_enabled_directory(&self) -> Result<bool, DbErr> {
        let found = ldap_directory::Entity::find()
            .select_only()
            .column(Column::Id)
            .filter(Column::Enabled.eq(true))
            .into_tuple::<i64>()
            .one(&self.inner.db)
            .await?;
        Ok(found.is_some())
    }

    fn settings_of(
        &self,
        row: &ldap_directory::Model,
    ) -> Result<DirectorySettings, DirectoryError> {
        let password = match row.bind_password_enc.as_deref() {
            Some(sealed) => {
                let context = password_context(row.id);
                let plain = self
                    .inner
                    .crypto
                    .open(&context, sealed)
                    .map_err(DirectoryError::Crypto)?;
                let text = String::from_utf8(plain.to_vec())
                    .map_err(|_| DirectoryError::Crypto(crate::crypto::CryptoError::Malformed))?;
                Some(Zeroizing::new(text))
            }
            None => None,
        };
        Ok(DirectorySettings::from_row(row, password))
    }

    async fn write_password(
        &self,
        tx: &DatabaseTransaction,
        row: ldap_directory::Model,
        password: Option<&Zeroizing<String>>,
    ) -> Result<ldap_directory::Model, DirectoryError> {
        let sealed = match password {
            Some(plain) => Some(
                self.inner
                    .crypto
                    .seal(&password_context(row.id), plain.as_bytes())
                    .map_err(DirectoryError::Crypto)?,
            ),
            None => None,
        };
        let mut active: ldap_directory::ActiveModel = row.into();
        active.bind_password_enc = Set(sealed);
        Ok(active.update(tx).await?)
    }
}

/// Sealing context of a directory's bind password.
fn password_context(id: i64) -> String {
    format!("ldap_directories.bind_password_enc:{id}")
}

fn apply(row: &mut ldap_directory::ActiveModel, fields: Fields, now: TimestampMs) {
    row.name = Set(fields.name);
    row.url = Set(fields.url);
    row.start_tls = Set(fields.start_tls);
    row.tls_verify = Set(fields.tls_verify);
    row.bind_dn = Set(fields.bind_dn);
    row.user_base_dn = Set(fields.user_base_dn);
    row.user_filter = Set(fields.user_filter);
    row.username_attr = Set(fields.username_attr);
    row.email_attr = Set(fields.email_attr);
    row.display_name_attr = Set(fields.display_name_attr);
    row.group_base_dn = Set(fields.group_base_dn);
    row.group_filter = Set(fields.group_filter);
    row.admin_group_dn = Set(fields.admin_group_dn);
    row.enabled = Set(fields.enabled);
    row.updated_at = Set(now);
}

/// The row, locked for the transaction; refused when the configuration owns it.
async fn writable(
    tx: &DatabaseTransaction,
    id: i64,
) -> Result<ldap_directory::Model, DirectoryError> {
    let found = ldap_directory::Entity::find_by_id(id)
        .lock_exclusive()
        .one(tx)
        .await?;
    let row = found.ok_or(DirectoryError::NotFound)?;
    if row.managed_by == ManagedBy::Config {
        return Err(DirectoryError::ManagedByConfig);
    }
    Ok(row)
}

/// Unique violations by index: the single-enabled index names `enabled` on both backends
/// (`uq_ldap_directories_enabled` / `ldap_directories.enabled`); anything else is the name.
fn conflict(err: DbErr) -> DirectoryError {
    match err.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(message)) => {
            let field = if message.contains("enabled") {
                "enabled"
            } else {
                "name"
            };
            DirectoryError::Conflict { field }
        }
        _ => DirectoryError::Database(err),
    }
}
