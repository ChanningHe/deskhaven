//! The console's view of API tokens (M3-12): a paged list with the creating account, and
//! creation and revocation decided and audited by a [`TxGuard`] inside the write
//! transaction (the rule "who may revoke whose token" is the API layer's; the guard sees
//! the token and its creator's committed role).

use deskhaven_db::begin_write;
use deskhaven_db::entity::api_token;
use deskhaven_db::entity::user::{self, Role};
use sea_orm::sea_query::{Expr, ExprTrait, Func, LockType};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, Order, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect,
};

use super::{ApiToken, ApiTokenError, ApiTokenService};
use crate::console_audit::TxGuard;
use crate::pagination::{Page, PageRequest};
use crate::text::contains_pattern;
use crate::users::lock_active_owners;

/// What a [`TxGuard`] of a token write sees: the token and its creator's committed role
/// (`None`: a system token without creator).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenTarget {
    pub token: ApiToken,
    pub creator_role: Option<Role>,
}

/// The account that created a token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenCreator {
    pub id: i64,
    pub username: String,
    pub role: Role,
}

/// A stored token with its creator, for listings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiTokenEntry {
    pub token: ApiToken,
    /// `None` for a system token.
    pub creator: Option<TokenCreator>,
}

impl From<(api_token::Model, Option<user::Model>)> for ApiTokenEntry {
    fn from((token, creator): (api_token::Model, Option<user::Model>)) -> Self {
        Self {
            token: token.into(),
            creator: creator.map(TokenCreator::from),
        }
    }
}

impl From<user::Model> for TokenCreator {
    fn from(u: user::Model) -> Self {
        Self {
            id: u.id,
            username: u.username,
            role: u.role,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApiTokenSortField {
    Name,
    CreatedAt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApiTokenSort {
    pub field: ApiTokenSortField,
    pub descending: bool,
}

/// Filters of [`ApiTokenService::page`]: `q` matches a substring of the name,
/// case-insensitively (ASCII on SQLite) and literally. Default order: newest first.
#[derive(Clone, Debug, Default)]
pub struct ApiTokenQuery {
    pub q: Option<String>,
    pub sort: Option<ApiTokenSort>,
    pub page: PageRequest,
}

impl ApiTokenService {
    /// One page of tokens, revoked and expired ones included.
    pub async fn page(&self, query: &ApiTokenQuery) -> Result<Page<ApiTokenEntry>, ApiTokenError> {
        let index = query.page.index().map_err(|p| ApiTokenError::Invalid {
            field: p.field,
            reason: p.reason,
        })?;
        let mut select = api_token::Entity::find().find_also_related(user::Entity);
        if let Some(q) = query.q.as_deref().filter(|q| !q.is_empty()) {
            let name = Expr::col((api_token::Entity, api_token::Column::Name));
            let name = Expr::expr(Func::lower(name));
            select = select.filter(name.like(contains_pattern(&q.to_lowercase())));
        }
        let sort = query.sort.unwrap_or(ApiTokenSort {
            field: ApiTokenSortField::CreatedAt,
            descending: true,
        });
        let column = match sort.field {
            ApiTokenSortField::Name => api_token::Column::Name,
            ApiTokenSortField::CreatedAt => api_token::Column::CreatedAt,
        };
        let order = match sort.descending {
            true => Order::Desc,
            false => Order::Asc,
        };
        let paginator = select
            .order_by(column, order.clone())
            .order_by(api_token::Column::Id, order)
            .paginate(&self.db, query.page.page_size);
        let total = paginator.num_items().await?;
        let rows = paginator.fetch_page(index).await?;
        Ok(Page {
            items: rows.into_iter().map(ApiTokenEntry::from).collect(),
            total,
            page: query.page.page,
            page_size: query.page.page_size,
        })
    }

    /// Token `id` with its creator.
    pub async fn entry(&self, id: i64) -> Result<Option<ApiTokenEntry>, ApiTokenError> {
        let found = api_token::Entity::find_by_id(id)
            .find_also_related(user::Entity)
            .one(&self.db)
            .await?;
        Ok(found.map(ApiTokenEntry::from))
    }

    /// [`revoke`](Self::revoke) with `guard` deciding on the token before and after, and
    /// recording the audit row in the same transaction. The guard sees the creator's
    /// committed role. Revoking a revoked token changes and records nothing. `false`: no
    /// such token.
    ///
    /// Locks are taken in the order `UserService::edit` takes them: all active owners, then
    /// the creator, then the acting account (in the guard), the token last. Disabling or
    /// demoting the creator revokes its tokens under that order, so the two cannot
    /// deadlock. Two revocations racing: the second finds the token revoked and rolls its
    /// audit row back with it.
    pub async fn revoke_guarded(
        &self,
        id: i64,
        guard: TxGuard<TokenTarget>,
    ) -> Result<bool, ApiTokenError> {
        let now = self.now()?;
        let tx = begin_write(&self.db).await?;
        let found = api_token::Entity::find_by_id(id).one(&tx).await?;
        let Some(row) = found else {
            return Ok(false);
        };
        if row.revoked_at.is_some() {
            return Ok(true);
        }
        lock_active_owners(&tx).await?;
        let creator_role = creator_role(&tx, row.created_by).await?;
        let before = TokenTarget {
            token: row.clone().into(),
            creator_role,
        };
        let revoked = api_token::Model {
            revoked_at: Some(now),
            ..row
        };
        let after = TokenTarget {
            token: revoked.into(),
            creator_role,
        };
        guard.check(&tx, Some(&before), Some(&after)).await?;
        let updated = api_token::Entity::update_many()
            .col_expr(api_token::Column::RevokedAt, Expr::value(now))
            .filter(api_token::Column::Id.eq(id))
            .filter(api_token::Column::RevokedAt.is_null())
            .exec(&tx)
            .await?;
        if updated.rows_affected == 0 {
            // Revoked meanwhile: dropping the transaction takes the audit row back too.
            return Ok(true);
        }
        tx.commit().await?;
        Ok(true)
    }
}

/// The committed role of the creating account, key-share locked like `ensure_creator`.
async fn creator_role<C: ConnectionTrait>(
    conn: &C,
    created_by: Option<i64>,
) -> Result<Option<Role>, ApiTokenError> {
    let Some(id) = created_by else {
        return Ok(None);
    };
    let found = user::Entity::find_by_id(id)
        .lock(LockType::KeyShare)
        .one(conn)
        .await?;
    Ok(found.map(|u| u.role))
}
