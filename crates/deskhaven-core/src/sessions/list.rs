//! A user's usable sessions: the paged list, and the per-user cap
//! (`auth.max_sessions_per_user`) that creation enforces by revoking the least recently
//! used ones.
//!
//! The cap is enforced in the creating transaction, which share-locks the user row only, so
//! concurrent creations for one user can leave it exceeded by one each until the next
//! creation; it bounds growth, not an exact count.

use deskhaven_config::settings::AUTH_MAX_SESSIONS_PER_USER;
use deskhaven_db::entity::session;
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, DbErr, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, Select,
};

use super::revoke::revoke_rows;
use super::{Core, Session, SessionError, SessionService, timestamp};
use crate::pagination::{Page, PageRequest};

/// Most excess sessions one creation revokes (a user far above a lowered cap is brought
/// down over several creations, in short statements).
const EVICT_BATCH: u64 = 500;

impl SessionService {
    /// One page of the user's usable sessions (unrevoked, within both expiries), most
    /// recently used first.
    pub async fn list_for_user(
        &self,
        user_id: i64,
        page: PageRequest,
    ) -> Result<Page<Session>, SessionError> {
        let index = page.index().map_err(|p| SessionError::Invalid {
            field: p.field,
            reason: p.reason,
        })?;
        let core = &self.core;
        let paginator = core
            .usable_of(user_id, core.now_ms())?
            .paginate(&core.db, page.page_size);
        let total = paginator.num_items().await?;
        let rows = paginator.fetch_page(index).await?;
        Ok(Page {
            items: rows.into_iter().map(Session::from).collect(),
            total,
            page: page.page,
            page_size: page.page_size,
        })
    }
}

impl Core {
    /// The usable sessions of `user_id` at `now`, most recently used first.
    fn usable_of(&self, user_id: i64, now: i64) -> Result<Select<session::Entity>, DbErr> {
        let usable = Condition::all()
            .add(session::Column::UserId.eq(user_id))
            .add(session::Column::RevokedAt.is_null())
            .add(session::Column::ExpiresAt.gt(timestamp(now)?))
            .add(session::Column::LastUsedAt.gt(timestamp(self.idle_cutoff(now))?));
        Ok(session::Entity::find()
            .filter(usable)
            .order_by_desc(session::Column::LastUsedAt)
            .order_by_desc(session::Column::Id))
    }

    /// Revokes the user's usable sessions beyond the cap, least recently used first (a
    /// session created at `now` is the most recent and stays); returns their ids, for the
    /// cache.
    pub(super) async fn evict_beyond_cap<C: ConnectionTrait>(
        &self,
        conn: &C,
        user_id: i64,
        now: i64,
    ) -> Result<Vec<i64>, DbErr> {
        let cap = self.settings.snapshot().get(&AUTH_MAX_SESSIONS_PER_USER);
        let excess: Vec<i64> = self
            .usable_of(user_id, now)?
            .select_only()
            .column(session::Column::Id)
            .offset(u64::from(cap))
            .limit(EVICT_BATCH)
            .into_tuple()
            .all(conn)
            .await?;
        if !excess.is_empty() {
            let which = session::Column::Id.is_in(excess.iter().copied()).into();
            revoke_rows(conn, which, timestamp(now)?).await?;
        }
        Ok(excess)
    }
}
