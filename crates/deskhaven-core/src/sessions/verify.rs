//! Token validation: cache, then database (with the owner's current status), then the
//! throttled `last_used_at` touch.

use deskhaven_db::entity::session;
use deskhaven_db::entity::user::{self, Status};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use super::model::binding;
use super::{
    CacheGeneration, CachedSession, Core, SessionError, SessionPrincipal, SessionService,
    TOUCH_INTERVAL, TokenHash, millis, timestamp,
};

impl SessionService {
    /// The principal of a presented token.
    ///
    /// Unauthenticated outcomes (see [`SessionError::is_unauthenticated`]): malformed or
    /// unknown token, revoked, absolutely or idle expired, owner not active or deleted.
    /// Database failures are [`SessionError::Database`].
    pub async fn authenticate(&self, token: &str) -> Result<SessionPrincipal, SessionError> {
        let hash = TokenHash::of_presented(token).ok_or(SessionError::InvalidToken)?;
        let core = &self.core;
        let now = core.now_ms();
        let cached = core.cache.get(&hash, now);
        let (session, fill) = match cached {
            Some(session) => (session, None),
            None => {
                let (session, seen) = core.load(&hash).await?;
                (session, Some(seen))
            }
        };
        if let Err(expired) = session.check(now, core.idle_ttl_ms()) {
            // Expiry is permanent, so this needs no fill-race protection (no generation
            // bump: a client repeating an expired token must not disturb other fills).
            core.cache.remove_expired(&hash);
            return Err(expired);
        }
        if let Some(seen) = fill {
            core.cache.insert(hash.clone(), session.clone(), seen, now);
        }
        core.touch_if_due(&hash, &session, now).await?;
        Ok(session.principal)
    }
}

impl Core {
    /// The session and its owner from the database, plus the cache generation seen before
    /// the read (for a race-free fill).
    async fn load(
        &self,
        hash: &TokenHash,
    ) -> Result<(CachedSession, CacheGeneration), SessionError> {
        let seen = self.cache.generation();
        let found = session::Entity::find()
            .filter(session::Column::TokenHash.eq(hash.as_str()))
            .find_also_related(user::Entity)
            .one(&self.db)
            .await?;
        let (row, owner) = found.ok_or(SessionError::InvalidToken)?;
        if row.revoked_at.is_some() {
            return Err(SessionError::Revoked);
        }
        let owner = owner
            .filter(|u| u.status == Status::Active)
            .ok_or(SessionError::UserInactive)?;
        let session = CachedSession {
            principal: SessionPrincipal {
                session_id: row.id,
                user_id: owner.id,
                role: owner.role,
                kind: row.kind,
                device: binding(row.rustdesk_id, row.device_uuid),
            },
            expires_at_ms: row.expires_at.timestamp_millis(),
            last_used_at_ms: row.last_used_at.timestamp_millis(),
        };
        Ok((session, seen))
    }

    /// Writes `last_used_at = now` when the recorded use is at least [`TOUCH_INTERVAL`] old.
    /// The update only matches an unrevoked row, so it doubles as a revocation re-check.
    async fn touch_if_due(
        &self,
        hash: &TokenHash,
        session: &CachedSession,
        now: i64,
    ) -> Result<(), SessionError> {
        if now.saturating_sub(session.last_used_at_ms) < millis(TOUCH_INTERVAL) {
            return Ok(());
        }
        let result = session::Entity::update_many()
            .col_expr(session::Column::LastUsedAt, Expr::value(timestamp(now)?))
            .filter(session::Column::Id.eq(session.principal.session_id))
            .filter(session::Column::RevokedAt.is_null())
            .exec(&self.db)
            .await?;
        if result.rows_affected == 0 {
            self.cache.invalidate_token(hash);
            return Err(SessionError::Revoked);
        }
        self.cache.mark_used(hash, now);
        Ok(())
    }
}
