//! Revocation, listing and cleanup.

use deskhaven_db::begin_write;
use deskhaven_db::entity::{session, user};
use deskhaven_db::time::TimestampMs;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, DbErr, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect,
};

use super::{Core, Session, SessionError, SessionService, TokenHash, detached, timestamp};
use crate::console_audit::TxGuard;
use crate::users::User;

/// Rows deleted per statement by [`SessionService::cleanup`] (short write locks).
const CLEANUP_BATCH: u64 = 500;

/// Sets `revoked_at` on every unrevoked session of `user_id`. For account changes that must
/// revoke in their own transaction (`users::write`); the caller's [`UserEvent`] then
/// invalidates the cache.
///
/// [`UserEvent`]: crate::users::UserEvent
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
    /// Revokes one session. `false` when it does not exist or was already revoked.
    pub async fn revoke(&self, session_id: i64) -> Result<bool, SessionError> {
        let core = self.core.clone();
        detached(async move {
            let revoked = core.revoke(session::Column::Id.eq(session_id).into()).await;
            core.cache.invalidate_session(session_id);
            Ok(revoked? > 0)
        })
        .await
    }

    /// Logout: revokes the session of a presented token. `false` when the token is
    /// malformed, unknown or already revoked.
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

    /// Revokes every session of `user_id`; returns how many were still active.
    pub async fn revoke_all_for_user(&self, user_id: i64) -> Result<u64, SessionError> {
        let core = self.core.clone();
        detached(async move { Ok(core.revoke_all_for_user(user_id).await?) }).await
    }

    /// [`revoke_all_for_user`](Self::revoke_all_for_user) by an administrator, in one
    /// transaction over the locked user row, with `guard` deciding on the account (the same
    /// account as `before` and `after`). [`SessionError::UserNotFound`] for an unknown user.
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

    /// The user's usable sessions (unrevoked, within both expiries), most recently used
    /// first.
    pub async fn list_for_user(&self, user_id: i64) -> Result<Vec<Session>, SessionError> {
        let core = &self.core;
        let now = core.now_ms();
        let rows = session::Entity::find()
            .filter(session::Column::UserId.eq(user_id))
            .filter(session::Column::RevokedAt.is_null())
            .filter(session::Column::ExpiresAt.gt(timestamp(now)?))
            .filter(session::Column::LastUsedAt.gt(timestamp(core.idle_cutoff(now))?))
            .order_by_desc(session::Column::LastUsedAt)
            .order_by_desc(session::Column::Id)
            .all(&core.db)
            .await?;
        Ok(rows.into_iter().map(Session::from).collect())
    }

    /// Deletes revoked and expired (absolute or idle) sessions in batches; returns the
    /// number deleted. Scheduled by the background jobs (M2-20).
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

    /// Cache first (nothing cached survives even a failing write), then the database, then
    /// the cache again (drops a fill that read the rows just before the write committed).
    pub(super) async fn revoke_all_for_user(&self, user_id: i64) -> Result<u64, DbErr> {
        self.cache.invalidate_user(user_id);
        let revoked = self
            .revoke(session::Column::UserId.eq(user_id).into())
            .await;
        self.cache.invalidate_user(user_id);
        revoked
    }

    /// Like [`revoke_all_for_user`](Self::revoke_all_for_user), but spares the sessions
    /// created at or after the user's last password change: the change itself already
    /// revoked the older ones in its transaction, and a session minted after it (the user
    /// signing in again, or the self-service change handing out a fresh one) is legitimate
    /// even when this late, asynchronous follow-up runs after it. A missing user (deleted)
    /// revokes everything.
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

    /// Sessions last used at or before this instant are idle-expired.
    pub(super) fn idle_cutoff(&self, now: i64) -> i64 {
        now.saturating_sub(self.idle_ttl_ms())
    }
}
