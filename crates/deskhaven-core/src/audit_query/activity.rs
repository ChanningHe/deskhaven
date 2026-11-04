//! What the overview page needs of the audit trail (M3-14): counts over a time window and
//! the latest alarms, without the count over the whole table a paged list makes.

use deskhaven_db::entity::alarm;
use deskhaven_db::entity::connection_session;
use deskhaven_db::entity::login_event;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect};

use super::{AuditQuery, AuditQueryError};
use crate::login_events::LoginResult;

/// Audit counts since a point in time, see [`AuditQuery::activity_since`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Activity {
    /// Connections that started in the window.
    pub connections: u64,
    /// Login attempts in the window that failed ([`LoginResult::NOT_FAILURES`] excluded).
    pub failed_logins: u64,
}

impl AuditQuery {
    /// Counts of the rows created at or after `since` (unix ms); each is a range scan of the
    /// `created_at` index.
    pub async fn activity_since(&self, since: i64) -> Result<Activity, AuditQueryError> {
        let connections = connection_session::Entity::find()
            .filter(connection_session::Column::CreatedAt.gte(since))
            .count(&self.db)
            .await?;
        let succeeded = LoginResult::NOT_FAILURES.map(LoginResult::as_str);
        let failed_logins = login_event::Entity::find()
            .filter(login_event::Column::CreatedAt.gte(since))
            .filter(login_event::Column::Result.is_not_in(succeeded))
            .count(&self.db)
            .await?;
        Ok(Activity {
            connections,
            failed_logins,
        })
    }

    /// The `limit` newest alarms, newest first.
    pub async fn latest_alarms(&self, limit: u64) -> Result<Vec<alarm::Model>, AuditQueryError> {
        Ok(alarm::Entity::find()
            .order_by_desc(alarm::Column::CreatedAt)
            .order_by_desc(alarm::Column::Id)
            .limit(limit)
            .all(&self.db)
            .await?)
    }
}
