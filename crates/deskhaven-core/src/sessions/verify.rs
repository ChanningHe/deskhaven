//! Token validation: cache, then database (with the owner's current status), then the
//! throttled `last_used_at` touch.

use deskhaven_db::entity::session;
use deskhaven_db::entity::user::{self, Status};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, DbErr, EntityTrait, QueryFilter};

use super::model::{AccountFlags, binding};
use super::{
    CacheGeneration, CachedSession, Core, SessionError, SessionKind, SessionPrincipal,
    SessionService, TOUCH_INTERVAL, TokenHash, millis, timestamp,
};

impl SessionService {
    /// The principal of a presented token, which must belong to a session of `expected`
    /// kind (a console token is no client credential, and vice versa).
    ///
    /// Unauthenticated outcomes (see [`SessionError::is_unauthenticated`]): malformed or
    /// unknown token, wrong kind, revoked, absolutely or idle expired, owner not active or
    /// deleted. Database failures reading the session are [`SessionError::Database`]; a
    /// failed `last_used_at` touch is only logged.
    pub async fn authenticate(
        &self,
        token: &str,
        expected: SessionKind,
    ) -> Result<SessionPrincipal, SessionError> {
        let hash = TokenHash::of_presented(token).ok_or(SessionError::InvalidToken)?;
        let core = &self.core;
        let now = core.now_ms();
        if let Some(session) = core.cache.get(&hash, now) {
            return core.validated(&hash, session, None, now, expected).await;
        }
        let (session, seen) = core.load(&hash).await?;
        core.validated(&hash, session, Some(seen), now, expected)
            .await
    }

    /// [`authenticate`](Self::authenticate) without the database read: the outcome when
    /// the token's session is in the cache (or the token is malformed), else
    /// [`CachedAuth::Miss`]. Lets callers rate-limit only the requests that would read the
    /// database. A hit may still write (the throttled `last_used_at` touch).
    pub async fn authenticate_cached(&self, token: &str, expected: SessionKind) -> CachedAuth {
        let Some(hash) = TokenHash::of_presented(token) else {
            return CachedAuth::Hit(Err(SessionError::InvalidToken));
        };
        let core = &self.core;
        let now = core.now_ms();
        match core.cache.get(&hash, now) {
            Some(session) => {
                CachedAuth::Hit(core.validated(&hash, session, None, now, expected).await)
            }
            None => CachedAuth::Miss,
        }
    }
}

/// Outcome of [`SessionService::authenticate_cached`].
#[derive(Debug)]
pub enum CachedAuth {
    /// Decided without reading the database.
    Hit(Result<SessionPrincipal, SessionError>),
    /// Not cached: only [`SessionService::authenticate`] can tell.
    Miss,
}

impl Core {
    /// Expiry, kind and touch checks of a session from the cache (`fill: None`) or the
    /// database (`fill`: the generation seen before the read, to cache it race-free).
    async fn validated(
        &self,
        hash: &TokenHash,
        session: CachedSession,
        fill: Option<CacheGeneration>,
        now: i64,
        expected: SessionKind,
    ) -> Result<SessionPrincipal, SessionError> {
        let core = self;
        if let Err(expired) = session.check(now, core.idle_ttl_ms()) {
            // Expiry is permanent, so this needs no fill-race protection (no generation
            // bump: a client repeating an expired token must not disturb other fills).
            core.cache.remove_expired(hash);
            return Err(expired);
        }
        if let Some(seen) = fill {
            core.cache.insert(hash.clone(), session.clone(), seen, now);
        }
        if session.principal.kind != expected {
            return Err(SessionError::InvalidToken);
        }
        core.touch_if_due(hash, &session, now).await?;
        Ok(session.principal)
    }
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
                account: AccountFlags::of(&owner),
            },
            expires_at_ms: row.expires_at.timestamp_millis(),
            last_used_at_ms: row.last_used_at.timestamp_millis(),
        };
        Ok((session, seen))
    }

    /// Writes `last_used_at = now` when the recorded use is at least [`TOUCH_INTERVAL`] old.
    /// The update only matches an unrevoked row, so it doubles as a revocation re-check: no
    /// row is [`SessionError::Revoked`]. A failed write is logged and the request allowed
    /// (the session was just validated; the next request retries the touch).
    async fn touch_if_due(
        &self,
        hash: &TokenHash,
        session: &CachedSession,
        now: i64,
    ) -> Result<(), SessionError> {
        if now.saturating_sub(session.last_used_at_ms) < millis(TOUCH_INTERVAL) {
            return Ok(());
        }
        let session_id = session.principal.session_id;
        match self.touch(session_id, now).await {
            Ok(0) => {
                self.cache.invalidate_token(hash);
                Err(SessionError::Revoked)
            }
            Ok(_) => {
                self.cache.mark_used(hash, now);
                Ok(())
            }
            Err(err) => {
                tracing::warn!(session_id, error = %err, "cannot record session use; allowing the request");
                Ok(())
            }
        }
    }

    async fn touch(&self, session_id: i64, now: i64) -> Result<u64, DbErr> {
        let result = session::Entity::update_many()
            .col_expr(session::Column::LastUsedAt, Expr::value(timestamp(now)?))
            .filter(session::Column::Id.eq(session_id))
            .filter(session::Column::RevokedAt.is_null())
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }
}
