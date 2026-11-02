//! First-run setup (05 §2.4, 04 §3 `setup_tokens`): while the instance has no owner, a
//! one-time link creates it.
//!
//! - **Token**: 256 random bits in the session-token format; only its SHA-256 is stored,
//!   with `expires_at` ([`SETUP_TOKEN_TTL`]) and `used_at`. At most one token is live:
//!   issuing one retires every unused token. The raw token leaves the service once, at
//!   issue (startup log or `deskhaven admin create-setup-link`).
//! - **Check**: the presented token is hashed and compared, in constant time, with every
//!   live row (normally one), so neither the database index nor the comparison times the
//!   token; a malformed token is rejected without a lookup.
//! - **Completion**: one write transaction claims the token (`used_at` set where it is
//!   still NULL: single use, also against concurrent requests), refuses when an owner
//!   already exists, creates the active local owner, retires every other token and records
//!   the caller's audit row. Any failure rolls all of it back.

use std::fmt;
use std::time::Duration;

use deskhaven_db::begin_write;
use deskhaven_db::entity::setup_token::{self, Column};
use deskhaven_db::entity::user;
use deskhaven_db::time::{self, TimestampMs};
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait,
    PaginatorTrait, QueryFilter,
};
use subtle::ConstantTimeEq;

use crate::clock::SharedClock;
use crate::console_audit::{self, NewConsoleAudit};
use crate::crypto::CryptoError;
use crate::sessions::{SessionToken, TokenHash};
use crate::users::{
    Credential, CredentialStamp, NewUser, Password, Role, Status, User, UserError, UserService,
};

/// Lifetime of a setup link.
pub const SETUP_TOKEN_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// A freshly issued setup token. Exposed once; `Debug` never shows it.
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

/// What startup did about the first-run setup.
#[derive(Debug)]
pub enum StartupSetup {
    /// An owner exists: nothing to set up.
    Done,
    /// No owner, and a token issued earlier is still live (its raw value is gone: reissue
    /// with `deskhaven admin create-setup-link` when the link was lost).
    Pending { expires_at: TimestampMs },
    /// No owner and no live token: this one was issued.
    Issued(SetupToken),
}

/// The owner the setup creates.
#[derive(Debug)]
pub struct NewOwner {
    pub username: String,
    /// Blank: the username.
    pub display_name: String,
    pub email: Option<String>,
    pub password: Password,
}

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    /// Unknown, expired, used or malformed token.
    #[error("invalid or expired setup token")]
    InvalidToken,
    #[error("the instance already has an owner")]
    OwnerExists,
    /// The account was refused (invalid username or email, password policy, ...).
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

/// The created owner and the stamp to sign them in under.
#[derive(Debug)]
pub struct SetupDone {
    pub user: User,
    pub stamp: CredentialStamp,
}

/// First-run setup tokens and completion. Cheap to clone.
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

    /// Whether the instance still needs its first owner.
    pub async fn is_required(&self) -> Result<bool, SetupError> {
        Ok(!owner_exists(&self.db).await?)
    }

    /// Issues a token unless an owner exists or a live token waits; see [`StartupSetup`].
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

    /// Issues a new token, retiring every unused one. [`SetupError::OwnerExists`] once
    /// the setup is done.
    pub async fn issue(&self) -> Result<SetupToken, SetupError> {
        issue(&self.db, self.clock.now_ms()).await
    }

    /// Creates the owner with the live token `presented` (see the module docs). `audit`
    /// builds the console audit row from the new owner; it is written in the same
    /// transaction.
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
        let tx = begin_write(&self.db).await?;
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

/// [`SetupService::issue`] on `db` at `now_ms`, for the CLI (no running service).
pub async fn issue(db: &DatabaseConnection, now_ms: i64) -> Result<SetupToken, SetupError> {
    let token = SessionToken::generate().map_err(SetupError::Random)?;
    let ttl = i64::try_from(SETUP_TOKEN_TTL.as_millis()).unwrap_or(i64::MAX);
    let expires_at = timestamp(now_ms.saturating_add(ttl))?;
    let tx = begin_write(db).await?;
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

/// The id of the live token whose hash equals `hash`, compared in constant time.
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

/// Marks every unused token used (retired).
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
