//! API tokens (06 §3.2, 04 §3 `api_tokens`): credentials for automation, `rustdesk --deploy`
//! and `rustdesk --assign`.
//!
//! - **Form**: [`API_TOKEN_PREFIX`] + 256 bits from the OS CSPRNG, base64url unpadded. Only
//!   the SHA-256 (hex) is stored, plus a short display prefix; the raw token leaves the
//!   service once, at creation.
//! - **Authority**: a token carries its creator's. It authenticates only while unrevoked,
//!   unexpired, holding the requested [`Scope`], and while its creator (if any) exists, is
//!   `active` and still an administrator, checked on every use. Disabling the creator or
//!   demoting them to `user` also revokes their tokens durably, in the same transaction
//!   ([`revoke_created_by`], `users::admin`), so re-enabling or re-promoting the account
//!   brings none back. Deleting the creator deletes the token (foreign key). A token without
//!   creator is a system token.
//! - **Use**: `last_used_at` is written at most once per [`TOUCH_INTERVAL`].
//! - Tokens are looked up by hash on every use (no cache): their endpoints are rare, and a
//!   revocation therefore takes effect on the next request.
//!
//! Creation, listing and revocation are the console's (M3-12), which also audits them.

mod authenticate;
mod error;
mod scope;
mod secret;

use std::collections::BTreeSet;

use deskhaven_db::entity::api_token;
use deskhaven_db::time::{self, TimestampMs};
use sea_orm::ActiveValue::{NotSet, Set};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait,
    QueryFilter, QueryOrder, SqlErr,
};

pub use authenticate::ApiTokenPrincipal;
pub use error::ApiTokenError;
pub use scope::Scope;
pub use secret::{API_TOKEN_PREFIX, ApiTokenSecret, DISPLAY_PREFIX_LEN};

use crate::clock::SharedClock;
pub use crate::sessions::TOUCH_INTERVAL;
use crate::text;

/// Longest token name, in characters.
pub const NAME_MAX_LEN: usize = 64;

/// Sets `revoked_at` on every unrevoked token `user_id` created. For account changes that
/// take the creator's authority away (disable, demotion to `user`), in their own write
/// transaction; returns the number revoked.
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

/// What [`ApiTokenService::create`] needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewApiToken {
    /// Shown in the console; surrounding blanks are trimmed.
    pub name: String,
    /// At least one.
    pub scopes: BTreeSet<Scope>,
    /// The creating administrator; `None` for a system token.
    pub created_by: Option<i64>,
    /// `None`: never expires. Must be in the future.
    pub expires_at: Option<TimestampMs>,
}

/// A stored token, without its secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiToken {
    pub id: i64,
    pub name: String,
    /// The first [`DISPLAY_PREFIX_LEN`] characters of the token.
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

/// Issues, lists, revokes and checks API tokens. Cheap to clone.
#[derive(Clone)]
pub struct ApiTokenService {
    db: DatabaseConnection,
    clock: SharedClock,
}

impl ApiTokenService {
    /// Timestamps and expiry use `clock` (share it with the other services).
    pub fn new(db: DatabaseConnection, clock: SharedClock) -> Self {
        Self { db, clock }
    }

    /// Stores a new token and returns it with its secret, the only time the secret exists
    /// outside the caller's hands.
    pub async fn create(
        &self,
        new: NewApiToken,
    ) -> Result<(ApiToken, ApiTokenSecret), ApiTokenError> {
        let now = self.now()?;
        let name = validate(&new, now)?;
        let secret = ApiTokenSecret::generate().map_err(ApiTokenError::Random)?;
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
        let stored = row.insert(&self.db).await.map_err(creator_missing)?;
        Ok((stored.into(), secret))
    }

    /// Every token, revoked and expired ones included, oldest first.
    pub async fn list(&self) -> Result<Vec<ApiToken>, ApiTokenError> {
        let rows = api_token::Entity::find()
            .order_by_asc(api_token::Column::Id)
            .all(&self.db)
            .await?;
        Ok(rows.into_iter().map(ApiToken::from).collect())
    }

    /// Revokes token `id` (effective on its next use); revoking it again keeps the first
    /// revocation time. `false`: no such token.
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

/// The trimmed name, once the whole request is valid.
fn validate(new: &NewApiToken, now: TimestampMs) -> Result<String, ApiTokenError> {
    let name = new.name.trim();
    let name_problem = match name.is_empty() {
        true => Some("must not be blank".to_owned()),
        false => text::problem(name, NAME_MAX_LEN, false),
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
    match problem {
        Some((field, reason)) => Err(ApiTokenError::Invalid { field, reason }),
        None => Ok(name.to_owned()),
    }
}

fn creator_missing(err: DbErr) -> ApiTokenError {
    match err.sql_err() {
        Some(SqlErr::ForeignKeyConstraintViolation(_)) => ApiTokenError::CreatorNotFound,
        _ => ApiTokenError::Database(err),
    }
}
