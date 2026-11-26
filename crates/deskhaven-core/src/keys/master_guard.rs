//! A missing key is generated only while the database holds no account: every sealed value belongs
//! to an account or was written by one. With accounts and no key, generating one would orphan every
//! sealed value. For a key gone for good, configure a new one explicitly; nothing sealed under the
//! old key can be read again.

use std::path::{Path, PathBuf};

use deskhaven_config::Secret;
use deskhaven_db::entity::user;
use sea_orm::{ConnectionTrait, DbErr, EntityTrait, QuerySelect};

use super::{KeyError, Loaded, load_or_generate_master_key, master_key_path, read_master_key};
use crate::crypto::MasterKey;

#[derive(Debug, thiserror::Error)]
pub enum MasterKeyError {
    #[error(transparent)]
    Key(#[from] KeyError),
    #[error(
        "no master key: security.master_key is not set and {} does not exist, but the \
         database already holds accounts whose secrets were sealed with a key. Restore that \
         key (the security.master_key value or secret file, or {}). Only if it is lost for \
         good, configure a new one explicitly; every secret sealed under the old key is then \
         unreadable",
        path.display(),
        path.display()
    )]
    Missing { path: PathBuf },
    #[error("cannot check the database for existing accounts")]
    Database(#[from] DbErr),
}

pub async fn load_or_generate_master_key_for<C: ConnectionTrait>(
    configured: &Secret,
    data_dir: &Path,
    db: &C,
) -> Result<Loaded<MasterKey>, MasterKeyError> {
    if let Some(loaded) = read_master_key(configured, data_dir)? {
        return Ok(loaded);
    }
    if has_accounts(db).await? {
        return Err(MasterKeyError::Missing {
            path: master_key_path(data_dir),
        });
    }
    Ok(load_or_generate_master_key(configured, data_dir)?)
}

async fn has_accounts<C: ConnectionTrait>(db: &C) -> Result<bool, DbErr> {
    let any = user::Entity::find()
        .select_only()
        .column(user::Column::Id)
        .limit(1)
        .into_tuple::<i64>()
        .one(db)
        .await?;
    Ok(any.is_some())
}
