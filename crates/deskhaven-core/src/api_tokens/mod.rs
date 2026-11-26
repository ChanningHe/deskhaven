//! Only the SHA-256 of a token is stored; the raw token leaves the service once, at creation.
//!
//! A token carries its creator's authority, checked on every use. Disabling or demoting the
//! creator revokes its tokens in the same transaction, so re-enabling brings none back.
//! Tokens are looked up on every use (no cache), so a revocation takes effect at once.

mod admin;
mod authenticate;
mod error;
mod scope;
mod secret;

use std::collections::BTreeSet;

use deskhaven_db::begin_write;
use deskhaven_db::entity::{api_token, user};
use deskhaven_db::time::{self, TimestampMs};
use sea_orm::ActiveValue::{NotSet, Set};
use sea_orm::sea_query::{Expr, LockType};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbErr,
    EntityTrait, QueryFilter, QueryOrder, QuerySelect, SqlErr,
};

pub use admin::{
    ApiTokenEntry, ApiTokenQuery, ApiTokenSort, ApiTokenSortField, TokenCreator, TokenTarget,
};
pub use authenticate::ApiTokenPrincipal;
pub use error::ApiTokenError;
pub use scope::Scope;
pub use secret::{API_TOKEN_PREFIX, ApiTokenSecret, DISPLAY_PREFIX_LEN};

use crate::clock::SharedClock;
use crate::console_audit::TxGuard;
pub use crate::sessions::TOUCH_INTERVAL;
use crate::text;

pub const NAME_MAX_LEN: usize = 64;

/// "Never" is `None` (the console warns about it), not a far-away date.
pub const MAX_LIFETIME_DAYS: i64 = 3650;
const MAX_LIFETIME_MS: i64 = MAX_LIFETIME_DAYS * 24 * 60 * 60 * 1000;

pub fn expiry_from_millis(ms: i64) -> Option<TimestampMs> {
    time::from_millis(ms)
}

pub(crate) async fn revoke_created_by<C: ConnectionTrait>(
    conn: &C,
    user_id: i64,
    now: TimestampMs,
) -> Result<u64, DbErr> {
    let result = api_token::Entity::update_many()
        .col_expr(api_token::Column::RevokedAt, Expr::value(now))
        .filter(api_token::Column::CreatedBy.eq(user_id))
        .filter(api_token::Column::RevokedAt.is_null())
        .exec(conn)
        .await?;
    Ok(result.rows_affected)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewApiToken {
    pub name: String,
    pub scopes: BTreeSet<Scope>,
    pub created_by: Option<i64>,
    pub expires_at: Option<TimestampMs>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiToken {
    pub id: i64,
    pub name: String,
    pub token_prefix: String,
    pub scopes: BTreeSet<Scope>,
    pub created_by: Option<i64>,
    pub created_at: TimestampMs,
    pub expires_at: Option<TimestampMs>,
    pub last_used_at: Option<TimestampMs>,
    pub revoked_at: Option<TimestampMs>,
}

impl From<api_token::Model> for ApiToken {
    fn from(m: api_token::Model) -> Self {
        Self {
            id: m.id,
            name: m.name,
            token_prefix: m.token_prefix,
            scopes: scope::decode(&m.scopes),
            created_by: m.created_by,
            created_at: m.created_at,
            expires_at: m.expires_at,
            last_used_at: m.last_used_at,
            revoked_at: m.revoked_at,
        }
    }
}

#[derive(Clone)]
pub struct ApiTokenService {
    db: DatabaseConnection,
    clock: SharedClock,
}

impl ApiTokenService {
    pub fn new(db: DatabaseConnection, clock: SharedClock) -> Self {
        Self { db, clock }
    }

    pub async fn create(
        &self,
        new: NewApiToken,
    ) -> Result<(ApiToken, ApiTokenSecret), ApiTokenError> {
        let (entry, secret) = self.create_in_tx(new, TxGuard::none()).await?;
        Ok((entry.token, secret))
    }

    pub async fn create_guarded(
        &self,
        new: NewApiToken,
        guard: TxGuard<TokenTarget>,
    ) -> Result<(ApiTokenEntry, ApiTokenSecret), ApiTokenError> {
        self.create_in_tx(new, guard).await
    }

    async fn create_in_tx(
        &self,
        new: NewApiToken,
        guard: TxGuard<TokenTarget>,
    ) -> Result<(ApiTokenEntry, ApiTokenSecret), ApiTokenError> {
        let now = self.now()?;
        let name = validate(&new, now)?;
        let secret = ApiTokenSecret::generate().map_err(ApiTokenError::Random)?;
        let tx = begin_write(&self.db).await?;
        let creator = match new.created_by {
            Some(id) => Some(TokenCreator::from(ensure_creator(&tx, id).await?)),
            None => None,
        };
        let creator_role = creator.as_ref().map(|c| c.role);
        let row = api_token::ActiveModel {
            id: NotSet,
            name: Set(name),
            token_prefix: Set(secret.display_prefix()),
            token_hash: Set(secret.hash().as_str().to_owned()),
            scopes: Set(scope::encode(&new.scopes)),
            created_by: Set(new.created_by),
            created_at: Set(now),
            expires_at: Set(new.expires_at),
            last_used_at: Set(None),
            revoked_at: Set(None),
        };
        let stored: ApiToken = row.insert(&tx).await.map_err(creator_missing)?.into();
        let target = TokenTarget {
            token: stored.clone(),
            creator_role,
        };
        guard.check(&tx, None, Some(&target)).await?;
        tx.commit().await?;
        let entry = ApiTokenEntry {
            token: stored,
            creator,
        };
        Ok((entry, secret))
    }

    pub async fn list(&self) -> Result<Vec<ApiToken>, ApiTokenError> {
        let rows = api_token::Entity::find()
            .order_by_asc(api_token::Column::Id)
            .all(&self.db)
            .await?;
        Ok(rows.into_iter().map(ApiToken::from).collect())
    }

    pub async fn revoke(&self, id: i64) -> Result<bool, ApiTokenError> {
        let now = self.now()?;
        let updated = api_token::Entity::update_many()
            .col_expr(api_token::Column::RevokedAt, Expr::value(now))
            .filter(api_token::Column::Id.eq(id))
            .filter(api_token::Column::RevokedAt.is_null())
            .exec(&self.db)
            .await?;
        if updated.rows_affected > 0 {
            return Ok(true);
        }
        let found = api_token::Entity::find_by_id(id).one(&self.db).await?;
        Ok(found.is_some())
    }

    fn now(&self) -> Result<TimestampMs, DbErr> {
        let ms = self.clock.now_ms();
        time::from_millis(ms).ok_or_else(|| DbErr::Custom(format!("timestamp {ms} out of range")))
    }
}

fn validate(new: &NewApiToken, now: TimestampMs) -> Result<String, ApiTokenError> {
    let name = new.name.trim();
    let name_problem = match name.is_empty() {
        true => Some("must not be blank".to_owned()),
        // Bidi controls and zero-width characters make look-alike names in the console.
        false => text::problem(name, NAME_MAX_LEN, false).or_else(|| {
            let hidden = name.chars().any(text::is_dropped);
            hidden.then(|| "must not contain control or invisible characters".to_owned())
        }),
    };
    let problem = name_problem.map(|reason| ("name", reason)).or_else(|| {
        let none = new.scopes.is_empty();
        none.then(|| ("scopes", "must name at least one scope".to_owned()))
    });
    let problem = problem.or_else(|| {
        let past = new
            .expires_at
            .is_some_and(|at| at.timestamp_millis() <= now.timestamp_millis());
        past.then(|| ("expires_at", "must be in the future".to_owned()))
    });
    let problem = problem.or_else(|| {
        let limit = now.timestamp_millis().saturating_add(MAX_LIFETIME_MS);
        let far = new
            .expires_at
            .is_some_and(|at| at.timestamp_millis() > limit);
        far.then(|| {
            let reason =
                format!("must be at most {MAX_LIFETIME_DAYS} days away; omit it for no expiry");
            ("expires_at", reason)
        })
    });
    match problem {
        Some((field, reason)) => Err(ApiTokenError::Invalid { field, reason }),
        None => Ok(name.to_owned()),
    }
}

/// The key-share lock makes a concurrent disable or demotion wait for this token, so its
/// revocation sees it.
async fn ensure_creator(
    tx: &DatabaseTransaction,
    creator: i64,
) -> Result<user::Model, ApiTokenError> {
    let found = user::Entity::find_by_id(creator)
        .lock(LockType::KeyShare)
        .one(tx)
        .await?;
    let user = found.ok_or(ApiTokenError::CreatorNotFound)?;
    match authenticate::is_active_admin(&user) {
        true => Ok(user),
        false => Err(ApiTokenError::CreatorInactive),
    }
}

fn creator_missing(err: DbErr) -> ApiTokenError {
    match err.sql_err() {
        Some(SqlErr::ForeignKeyConstraintViolation(_)) => ApiTokenError::CreatorNotFound,
        _ => ApiTokenError::Database(err),
    }
}
