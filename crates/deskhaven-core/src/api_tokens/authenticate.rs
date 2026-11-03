//! Token checks on every use: hash lookup, revocation, expiry, the creator's current status
//! and role, the scope, then the throttled `last_used_at` touch.

use std::collections::BTreeSet;

use deskhaven_db::entity::api_token;
use deskhaven_db::entity::user::{self, Role, Status};
use deskhaven_db::time::TimestampMs;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, DbErr, EntityTrait, QueryFilter};

use super::secret::presented_hash;
use super::{ApiTokenError, ApiTokenService, Scope, TOUCH_INTERVAL};

/// Who acts through an authenticated token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiTokenPrincipal {
    /// `api_tokens.id`: the audit actor.
    pub token_id: i64,
    /// The creating administrator; `None` for a system token.
    pub created_by: Option<i64>,
    pub scopes: BTreeSet<Scope>,
}

impl ApiTokenService {
    /// The principal of a presented token that holds `required` (see the module
    /// documentation for every condition). Unauthenticated outcomes are those of
    /// [`ApiTokenError::is_unauthenticated`]; database failures reading the token are
    /// [`ApiTokenError::Database`]; a failed `last_used_at` touch is only logged.
    pub async fn authenticate(
        &self,
        token: &str,
        required: Scope,
    ) -> Result<ApiTokenPrincipal, ApiTokenError> {
        let hash = presented_hash(token).ok_or(ApiTokenError::InvalidToken)?;
        let found = api_token::Entity::find()
            .filter(api_token::Column::TokenHash.eq(hash.as_str()))
            .find_also_related(user::Entity)
            .one(&self.db)
            .await?;
        let (row, creator) = found.ok_or(ApiTokenError::InvalidToken)?;
        let now = self.now()?;
        check(&row, creator.as_ref(), now)?;
        let scopes = super::scope::decode(&row.scopes);
        if !scopes.contains(&required) {
            return Err(ApiTokenError::MissingScope);
        }
        self.touch_if_due(&row, now).await?;
        Ok(ApiTokenPrincipal {
            token_id: row.id,
            created_by: row.created_by,
            scopes,
        })
    }

    /// Writes `last_used_at = now` when the recorded use is at least [`TOUCH_INTERVAL`] old.
    /// The update only matches an unrevoked row, so it doubles as a revocation re-check.
    async fn touch_if_due(
        &self,
        row: &api_token::Model,
        now: TimestampMs,
    ) -> Result<(), ApiTokenError> {
        let interval = i64::try_from(TOUCH_INTERVAL.as_millis()).unwrap_or(i64::MAX);
        let due = row.last_used_at.is_none_or(|used| {
            let elapsed = now
                .timestamp_millis()
                .saturating_sub(used.timestamp_millis());
            elapsed >= interval
        });
        if !due {
            return Ok(());
        }
        match self.touch(row.id, now).await {
            Ok(0) => Err(ApiTokenError::Revoked),
            Ok(_) => Ok(()),
            Err(err) => {
                tracing::warn!(token_id = row.id, error = %err, "cannot record API token use; allowing the request");
                Ok(())
            }
        }
    }

    async fn touch(&self, id: i64, now: TimestampMs) -> Result<u64, DbErr> {
        let result = api_token::Entity::update_many()
            .col_expr(api_token::Column::LastUsedAt, Expr::value(now))
            .filter(api_token::Column::Id.eq(id))
            .filter(api_token::Column::RevokedAt.is_null())
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }
}

/// Revocation, expiry and the creator, in that order.
fn check(
    row: &api_token::Model,
    creator: Option<&user::Model>,
    now: TimestampMs,
) -> Result<(), ApiTokenError> {
    if row.revoked_at.is_some() {
        return Err(ApiTokenError::Revoked);
    }
    if row
        .expires_at
        .is_some_and(|at| at.timestamp_millis() <= now.timestamp_millis())
    {
        return Err(ApiTokenError::Expired);
    }
    let Some(created_by) = row.created_by else {
        return Ok(());
    };
    let authorized = creator.is_some_and(|u| u.id == created_by && is_active_admin(u));
    match authorized {
        true => Ok(()),
        false => Err(ApiTokenError::CreatorInactive),
    }
}

/// An account whose authority a token can carry.
pub(super) fn is_active_admin(user: &user::Model) -> bool {
    user.status == Status::Active && matches!(user.role, Role::Owner | Role::Admin)
}
