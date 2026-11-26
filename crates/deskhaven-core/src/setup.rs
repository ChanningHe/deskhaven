//! Only the token's SHA-256 is stored; at most one token is live. The presented token is compared
//! in constant time with every live row, so neither the index nor the comparison times it. Issue
//! and completion are serialized across processes, so two completions can never both see "no owner
//! yet".

use std::fmt;
use std::time::Duration;

use deskhaven_db::begin_write;
use deskhaven_db::entity::setup_token::{self, Column};
use deskhaven_db::entity::user;
use deskhaven_db::time::{self, TimestampMs};
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, DatabaseTransaction,
    DbBackend, DbErr, EntityTrait, PaginatorTrait, QueryFilter,
};
use subtle::ConstantTimeEq;

use crate::clock::SharedClock;
use crate::console_audit::{self, NewConsoleAudit};
use crate::crypto::CryptoError;
use crate::sessions::{SessionToken, TokenHash};
use crate::users::{
    Credential, CredentialStamp, NewUser, Password, Role, Status, User, UserError, UserService,
};

pub const SETUP_TOKEN_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Exposed once; `Debug` never shows it.
pub struct SetupToken {
    token: SessionToken,
    pub expires_at: TimestampMs,
}

impl SetupToken {
    pub fn expose(&self) -> &str {
        self.token.expose()
    }
}

impl fmt::Debug for SetupToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SetupToken")
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub enum StartupSetup {
    Done,
    Pending { expires_at: TimestampMs },
    Issued(SetupToken),
}

#[derive(Debug)]
pub struct NewOwner {
    pub username: String,
    pub display_name: String,
    pub email: Option<String>,
    pub password: Password,
}

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("invalid or expired setup token")]
    InvalidToken,
    #[error("the instance already has an owner")]
    OwnerExists,
    #[error(transparent)]
    User(UserError),
    #[error("cannot generate a setup token")]
    Random(#[source] CryptoError),
    #[error("setup storage failed")]
    Db(#[from] DbErr),
}

impl From<UserError> for SetupError {
    fn from(err: UserError) -> Self {
        match err {
            UserError::Database(db) => Self::Db(db),
            other => Self::User(other),
        }
    }
}

#[derive(Debug)]
pub struct SetupDone {
    pub user: User,
    pub stamp: CredentialStamp,
}

#[derive(Clone)]
pub struct SetupService {
    db: DatabaseConnection,
    users: UserService,
    clock: SharedClock,
}

impl SetupService {
    pub fn new(db: DatabaseConnection, users: UserService, clock: SharedClock) -> Self {
        Self { db, users, clock }
    }

    pub async fn is_required(&self) -> Result<bool, SetupError> {
        Ok(!owner_exists(&self.db).await?)
    }

    pub async fn on_startup(&self) -> Result<StartupSetup, SetupError> {
        if owner_exists(&self.db).await? {
            return Ok(StartupSetup::Done);
        }
        let live = setup_token::Entity::find()
            .filter(Column::UsedAt.is_null())
            .filter(Column::ExpiresAt.gt(self.now()?))
            .one(&self.db)
            .await?;
        match live {
            Some(row) => Ok(StartupSetup::Pending {
                expires_at: row.expires_at,
            }),
            None => self.issue().await.map(StartupSetup::Issued),
        }
    }

    pub async fn issue(&self) -> Result<SetupToken, SetupError> {
        issue(&self.db, self.clock.now_ms()).await
    }

    pub async fn complete(
        &self,
        presented: &str,
        owner: NewOwner,
        audit: impl FnOnce(&User) -> NewConsoleAudit,
    ) -> Result<SetupDone, SetupError> {
        let hash = TokenHash::of_presented(presented).ok_or(SetupError::InvalidToken)?;
        let new = NewUser {
            display_name: owner.display_name,
            email: owner.email,
            role: Role::Owner,
            status: Status::Active,
            ..NewUser::new(
                owner.username,
                Credential::Local {
                    password: owner.password,
                },
            )
        };
        // Hashed before the write lock is taken.
        let prepared = self.users.prepare(&new).await?;
        let now = self.now()?;
        let tx = begin_setup(&self.db).await?;
        let token_id = matching_live_token(&tx, &hash, now).await?;
        if owner_exists(&tx).await? {
            return Err(SetupError::OwnerExists);
        }
        let claimed = setup_token::Entity::update_many()
            .col_expr(Column::UsedAt, Expr::value(now))
            .filter(Column::Id.eq(token_id))
            .filter(Column::UsedAt.is_null())
            .exec(&tx)
            .await?;
        if claimed.rows_affected != 1 {
            return Err(SetupError::InvalidToken);
        }
        retire_unused(&tx, now).await?;
        let created = self.users.insert_prepared(&tx, prepared).await?;
        let stamp = CredentialStamp::of(&created);
        let user = User::from(created);
        console_audit::insert(&tx, &audit(&user), now.timestamp_millis()).await?;
        tx.commit().await?;
        Ok(SetupDone { user, stamp })
    }

    fn now(&self) -> Result<TimestampMs, DbErr> {
        timestamp(self.clock.now_ms())
    }
}

pub async fn issue(db: &DatabaseConnection, now_ms: i64) -> Result<SetupToken, SetupError> {
    let token = SessionToken::generate().map_err(SetupError::Random)?;
    let ttl = i64::try_from(SETUP_TOKEN_TTL.as_millis()).unwrap_or(i64::MAX);
    let expires_at = timestamp(now_ms.saturating_add(ttl))?;
    let tx = begin_setup(db).await?;
    if owner_exists(&tx).await? {
        return Err(SetupError::OwnerExists);
    }
    retire_unused(&tx, timestamp(now_ms)?).await?;
    setup_token::ActiveModel {
        token_hash: Set(token.hash().as_str().to_owned()),
        created_at: Set(timestamp(now_ms)?),
        expires_at: Set(expires_at),
        used_at: Set(None),
        ..Default::default()
    }
    .insert(&tx)
    .await?;
    tx.commit().await?;
    Ok(SetupToken { token, expires_at })
}

const PG_SETUP_LOCK_KEY: i64 = 0x6468_7365_7475_7021;

async fn begin_setup(db: &DatabaseConnection) -> Result<DatabaseTransaction, DbErr> {
    let tx = begin_write(db).await?;
    if tx.get_database_backend() == DbBackend::Postgres {
        let sql = format!("SELECT pg_advisory_xact_lock({PG_SETUP_LOCK_KEY})");
        tx.execute_unprepared(&sql).await?;
    }
    Ok(tx)
}

async fn matching_live_token<C: ConnectionTrait>(
    conn: &C,
    hash: &TokenHash,
    now: TimestampMs,
) -> Result<i64, SetupError> {
    let live = setup_token::Entity::find()
        .filter(Column::UsedAt.is_null())
        .filter(Column::ExpiresAt.gt(now))
        .all(conn)
        .await?;
    let presented = hash.as_str().as_bytes();
    let mut found = None;
    for row in live {
        if bool::from(row.token_hash.as_bytes().ct_eq(presented)) {
            found = Some(row.id);
        }
    }
    found.ok_or(SetupError::InvalidToken)
}

async fn owner_exists<C: ConnectionTrait>(conn: &C) -> Result<bool, DbErr> {
    let owners = user::Entity::find()
        .filter(user::Column::Role.eq(Role::Owner))
        .count(conn)
        .await?;
    Ok(owners > 0)
}

async fn retire_unused<C: ConnectionTrait>(conn: &C, now: TimestampMs) -> Result<(), DbErr> {
    setup_token::Entity::update_many()
        .col_expr(Column::UsedAt, Expr::value(now))
        .filter(Column::UsedAt.is_null())
        .exec(conn)
        .await?;
    Ok(())
}

fn timestamp(ms: i64) -> Result<TimestampMs, DbErr> {
    time::from_millis(ms).ok_or_else(|| DbErr::Custom(format!("timestamp {ms} out of range")))
}
