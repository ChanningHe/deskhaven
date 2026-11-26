use deskhaven_db::begin_write;
use deskhaven_db::entity::{session, user};
use deskhaven_db::time::TimestampMs;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, DbErr, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect,
};

use super::{Core, SessionError, SessionService, TokenHash, detached, timestamp};
use crate::console_audit::TxGuard;
use crate::users::User;

/// Short write locks.
const CLEANUP_BATCH: u64 = 500;

pub(crate) async fn revoke_user_sessions<C: ConnectionTrait>(
    conn: &C,
    user_id: i64,
    now: TimestampMs,
) -> Result<u64, DbErr> {
    revoke_rows(conn, session::Column::UserId.eq(user_id).into(), now).await
}

pub(super) async fn revoke_rows<C: ConnectionTrait>(
    conn: &C,
    which: Condition,
    now: TimestampMs,
) -> Result<u64, DbErr> {
    let result = session::Entity::update_many()
        .col_expr(session::Column::RevokedAt, Expr::value(now))
        .filter(which)
        .filter(session::Column::RevokedAt.is_null())
        .exec(conn)
        .await?;
    Ok(result.rows_affected)
}

impl SessionService {
    pub async fn revoke(&self, session_id: i64) -> Result<bool, SessionError> {
        let core = self.core.clone();
        detached(async move {
            let revoked = core.revoke(session::Column::Id.eq(session_id).into()).await;
            core.cache.invalidate_session(session_id);
            Ok(revoked? > 0)
        })
        .await
    }

    pub async fn revoke_token(&self, token: &str) -> Result<bool, SessionError> {
        let Some(hash) = TokenHash::of_presented(token) else {
            return Ok(false);
        };
        let core = self.core.clone();
        detached(async move {
            let which = session::Column::TokenHash.eq(hash.as_str()).into();
            let revoked = core.revoke(which).await;
            core.cache.invalidate_token(&hash);
            Ok(revoked? > 0)
        })
        .await
    }

    pub async fn revoke_all_for_user(&self, user_id: i64) -> Result<u64, SessionError> {
        let core = self.core.clone();
        detached(async move { Ok(core.revoke_all_for_user(user_id).await?) }).await
    }

    pub async fn revoke_all_for_user_guarded(
        &self,
        user_id: i64,
        guard: TxGuard<User>,
    ) -> Result<u64, SessionError> {
        let core = self.core.clone();
        detached(async move {
            core.cache.invalidate_user(user_id);
            let revoked = core.revoke_all_guarded(user_id, guard).await;
            core.cache.invalidate_user(user_id);
            revoked
        })
        .await
    }

    pub async fn cleanup(&self) -> Result<u64, SessionError> {
        let core = &self.core;
        let now = core.now_ms();
        let dead = Condition::any()
            .add(session::Column::RevokedAt.is_not_null())
            .add(session::Column::ExpiresAt.lte(timestamp(now)?))
            .add(session::Column::LastUsedAt.lte(timestamp(core.idle_cutoff(now))?));
        let mut deleted = 0;
        loop {
            let ids: Vec<i64> = session::Entity::find()
                .select_only()
                .column(session::Column::Id)
                .filter(dead.clone())
                .order_by_asc(session::Column::Id)
                .limit(CLEANUP_BATCH)
                .into_tuple()
                .all(&core.db)
                .await?;
            if ids.is_empty() {
                return Ok(deleted);
            }
            let batch = u64::try_from(ids.len()).unwrap_or(u64::MAX);
            let result = session::Entity::delete_many()
                .filter(session::Column::Id.is_in(ids))
                .exec(&core.db)
                .await?;
            deleted += result.rows_affected;
            if batch < CLEANUP_BATCH {
                return Ok(deleted);
            }
        }
    }
}

impl Core {
    async fn revoke(&self, which: Condition) -> Result<u64, DbErr> {
        revoke_rows(&self.db, which, timestamp(self.now_ms())?).await
    }

    /// Cache first (nothing cached survives even a failing write), then the database, then the cache
    /// again (drops a fill that read the rows just before the commit).
    pub(super) async fn revoke_all_for_user(&self, user_id: i64) -> Result<u64, DbErr> {
        self.cache.invalidate_user(user_id);
        let revoked = self
            .revoke(session::Column::UserId.eq(user_id).into())
            .await;
        self.cache.invalidate_user(user_id);
        revoked
    }

    /// Spares sessions created at or after the last password change: the change already revoked the
    /// older ones, and a session minted after it is legitimate even when this asynchronous follow-up
    /// runs late.
    pub(super) async fn revoke_sessions_before_password_change(
        &self,
        user_id: i64,
    ) -> Result<u64, DbErr> {
        self.cache.invalidate_user(user_id);
        let changed_at = user::Entity::find_by_id(user_id)
            .select_only()
            .column(user::Column::PasswordChangedAt)
            .into_tuple::<Option<TimestampMs>>()
            .one(&self.db)
            .await?;
        let mut which = Condition::all().add(session::Column::UserId.eq(user_id));
        if let Some(Some(changed_at)) = changed_at {
            which = which.add(session::Column::CreatedAt.lt(changed_at));
        }
        let revoked = self.revoke(which).await;
        self.cache.invalidate_user(user_id);
        revoked
    }

    async fn revoke_all_guarded(
        &self,
        user_id: i64,
        guard: TxGuard<User>,
    ) -> Result<u64, SessionError> {
        let tx = begin_write(&self.db).await?;
        let found = user::Entity::find_by_id(user_id)
            .lock_exclusive()
            .one(&tx)
            .await?;
        let target = User::from(found.ok_or(SessionError::UserNotFound)?);
        guard.check(&tx, Some(&target), Some(&target)).await?;
        let revoked = revoke_user_sessions(&tx, user_id, timestamp(self.now_ms())?).await?;
        tx.commit().await?;
        Ok(revoked)
    }

    pub(super) fn idle_cutoff(&self, now: i64) -> i64 {
        now.saturating_sub(self.idle_ttl_ms())
    }
}
