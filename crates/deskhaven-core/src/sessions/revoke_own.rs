//! A user's revocation of their own sessions (the console's "my sessions"): the sessions
//! are matched by owner in the revoking statement itself, and the audit row commits with
//! the revocation.

use deskhaven_db::begin_write;
use deskhaven_db::entity::session;
use sea_orm::{ColumnTrait, Condition};

use super::revoke::revoke_rows;
use super::{Core, SessionError, SessionService, detached, timestamp};
use crate::console_audit::{self, NewConsoleAudit};

impl SessionService {
    /// Revokes the usable session `session_id` of `user_id` and records `audit` in the same
    /// transaction. `false` (nothing recorded) when `user_id` has no such session: unknown,
    /// already dead, or another user's.
    pub async fn revoke_own_audited(
        &self,
        user_id: i64,
        session_id: i64,
        audit: &NewConsoleAudit,
    ) -> Result<bool, SessionError> {
        let which = Condition::all()
            .add(session::Column::UserId.eq(user_id))
            .add(session::Column::Id.eq(session_id));
        Ok(self.revoke_recorded(user_id, which, audit).await? > 0)
    }

    /// Revokes every usable session of `user_id` except `keep` (the caller's own), recording
    /// `audit` in the same transaction when any was revoked; returns how many.
    pub async fn revoke_others_audited(
        &self,
        user_id: i64,
        keep: i64,
        audit: &NewConsoleAudit,
    ) -> Result<u64, SessionError> {
        let which = Condition::all()
            .add(session::Column::UserId.eq(user_id))
            .add(session::Column::Id.ne(keep));
        self.revoke_recorded(user_id, which, audit).await
    }

    async fn revoke_recorded(
        &self,
        user_id: i64,
        which: Condition,
        audit: &NewConsoleAudit,
    ) -> Result<u64, SessionError> {
        let core = self.core.clone();
        let audit = audit.clone();
        detached(async move {
            core.cache.invalidate_user(user_id);
            let revoked = core.revoke_usable(which, &audit).await;
            core.cache.invalidate_user(user_id);
            revoked
        })
        .await
    }
}

impl Core {
    /// Revokes the unrevoked, unexpired, non-idle sessions matching `which`.
    async fn revoke_usable(
        &self,
        which: Condition,
        audit: &NewConsoleAudit,
    ) -> Result<u64, SessionError> {
        let now = self.now_ms();
        let usable = Condition::all()
            .add(which)
            .add(session::Column::ExpiresAt.gt(timestamp(now)?))
            .add(session::Column::LastUsedAt.gt(timestamp(self.idle_cutoff(now))?));
        let tx = begin_write(&self.db).await?;
        let revoked = revoke_rows(&tx, usable, timestamp(now)?).await?;
        if revoked == 0 {
            // Dropping the transaction rolls it back: nothing changed, nothing recorded.
            return Ok(0);
        }
        console_audit::insert(&tx, audit, now).await?;
        tx.commit().await?;
        Ok(revoked)
    }
}
