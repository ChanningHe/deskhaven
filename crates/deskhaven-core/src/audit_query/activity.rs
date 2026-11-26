use deskhaven_db::entity::alarm;
use deskhaven_db::entity::connection_session;
use deskhaven_db::entity::login_event;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect};

use super::{AuditQuery, AuditQueryError};
use crate::login_events::LoginResult;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Activity {
    pub connections: u64,
    pub failed_logins: u64,
}

impl AuditQuery {
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

    pub async fn latest_alarms(&self, limit: u64) -> Result<Vec<alarm::Model>, AuditQueryError> {
        Ok(alarm::Entity::find()
            .order_by_desc(alarm::Column::CreatedAt)
            .order_by_desc(alarm::Column::Id)
            .limit(limit)
            .all(&self.db)
            .await?)
    }
}
