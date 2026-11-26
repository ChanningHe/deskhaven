//! A request that matches nothing costs only an unlocked read: no write transaction and no cache
//! invalidation, so such requests cannot be used to churn either.

use deskhaven_db::begin_write;
use deskhaven_db::entity::session;
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QuerySelect};
use serde_json::json;

use super::revoke::revoke_rows;
use super::{Core, SessionError, SessionService, detached, timestamp};
use crate::console_audit::{self, NewConsoleAudit};

impl SessionService {
    pub async fn revoke_own_audited(
        &self,
        user_id: i64,
        session_id: i64,
        audit: &NewConsoleAudit,
    ) -> Result<bool, SessionError> {
        let which = Condition::all()
            .add(session::Column::UserId.eq(user_id))
            .add(session::Column::Id.eq(session_id));
        let core = self.core.clone();
        let audit = audit.clone();
        detached(async move {
            let revoked = core.revoke_usable(which, &audit).await;
            if !matches!(revoked, Ok(0)) {
                core.cache.invalidate_session(session_id);
            }
            Ok(revoked? > 0)
        })
        .await
    }

    pub async fn revoke_others_audited(
        &self,
        user_id: i64,
        keep: i64,
        audit: &NewConsoleAudit,
    ) -> Result<u64, SessionError> {
        let which = Condition::all()
            .add(session::Column::UserId.eq(user_id))
            .add(session::Column::Id.ne(keep));
        let core = self.core.clone();
        let audit = audit.clone();
        detached(async move {
            let revoked = core.revoke_usable(which, &audit).await;
            if !matches!(revoked, Ok(0)) {
                core.cache.invalidate_user(user_id);
            }
            revoked
        })
        .await
    }
}

impl Core {
    async fn revoke_usable(
        &self,
        which: Condition,
        audit: &NewConsoleAudit,
    ) -> Result<u64, SessionError> {
        let now = self.now_ms();
        let usable = Condition::all()
            .add(which)
            .add(session::Column::ExpiresAt.gt(timestamp(now)?))
            .add(session::Column::LastUsedAt.gt(timestamp(self.idle_cutoff(now))?))
            .add(session::Column::RevokedAt.is_null());
        let probe: Option<i64> = session::Entity::find()
            .select_only()
            .column(session::Column::Id)
            .filter(usable.clone())
            .limit(1)
            .into_tuple()
            .one(&self.db)
            .await?;
        if probe.is_none() {
            return Ok(0);
        }
        let tx = begin_write(&self.db).await?;
        let revoked = revoke_rows(&tx, usable, timestamp(now)?).await?;
        if revoked == 0 {
            // Dropping the transaction rolls it back: nothing changed, nothing recorded.
            return Ok(0);
        }
        let mut audit = audit.clone();
        if audit.after.is_none() {
            audit.after = Some(json!({ "revoked": revoked }));
        }
        console_audit::insert(&tx, &audit, now).await?;
        tx.commit().await?;
        Ok(revoked)
    }
}
