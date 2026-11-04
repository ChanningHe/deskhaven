//! The console audit log (`console_audit`, 06 §7).

use deskhaven_db::entity::console_audit::{ActorKind, Column, Entity, Model};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::Value;

use super::listing::{Listing, text};
use super::names::{token_names, usernames};
use super::{AuditQuery, AuditQueryError};
use crate::pagination::Page;

/// Filters of [`AuditQuery::console`]. `q` matches the action, target type and target id.
#[derive(Clone, Debug, Default)]
pub struct ConsoleQuery {
    pub listing: Listing,
    /// Changes made by this user account (an API token's changes: `actor_kind`).
    pub user_id: Option<i64>,
    pub actor_kind: Option<ActorKind>,
    /// The action name exactly, e.g. `user.disable`.
    pub action: Option<String>,
    pub target_type: Option<String>,
    pub q: Option<String>,
}

/// Who made a change: the kind, the id for users and tokens, and the name while the
/// account or token still exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleActor {
    pub kind: ActorKind,
    pub id: Option<i64>,
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleEntry {
    pub row: Model,
    pub actor: ConsoleActor,
}

/// A stored `before` / `after` snapshot as JSON. The writer serializes it, so a column that
/// does not parse (a hand-edited database) is shown as the text it holds.
pub fn parse_snapshot(stored: Option<&str>) -> Option<Value> {
    let stored = stored?;
    Some(serde_json::from_str(stored).unwrap_or_else(|_| Value::String(stored.to_owned())))
}

impl AuditQuery {
    pub async fn console(
        &self,
        query: &ConsoleQuery,
    ) -> Result<Page<ConsoleEntry>, AuditQueryError> {
        let mut select = Entity::find();
        if let Some(user) = query.user_id {
            select = select
                .filter(Column::ActorKind.eq(ActorKind::User))
                .filter(Column::ActorId.eq(user));
        }
        if let Some(kind) = query.actor_kind {
            select = select.filter(Column::ActorKind.eq(kind));
        }
        if let Some(action) = &query.action {
            select = select.filter(Column::Action.eq(action.as_str()));
        }
        if let Some(target_type) = &query.target_type {
            select = select.filter(Column::TargetType.eq(target_type.as_str()));
        }
        if let Some(q) = text(query.q.as_deref()) {
            let columns = [Column::Action, Column::TargetType, Column::TargetId];
            select = select.filter(self.contains(&columns, q));
        }
        let mut page = query
            .listing
            .run(&self.db, select, (Column::CreatedAt, Column::Id))
            .await?;
        let rows = std::mem::take(&mut page.items);
        let entries = self.with_actors(rows).await?;
        Ok(page.with_items(entries))
    }

    async fn with_actors(&self, rows: Vec<Model>) -> Result<Vec<ConsoleEntry>, AuditQueryError> {
        let ids_of = |kind: ActorKind| {
            rows.iter()
                .filter(move |r| r.actor_kind == kind)
                .filter_map(|r| r.actor_id)
                .collect::<Vec<_>>()
        };
        let users = usernames(&self.db, ids_of(ActorKind::User)).await?;
        let tokens = token_names(&self.db, ids_of(ActorKind::ApiToken)).await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let names = match row.actor_kind {
                    ActorKind::User => Some(&users),
                    ActorKind::ApiToken => Some(&tokens),
                    ActorKind::Cli | ActorKind::System => None,
                };
                let name = row.actor_id.and_then(|id| names?.get(&id)).cloned();
                let actor = ConsoleActor {
                    kind: row.actor_kind,
                    id: row.actor_id,
                    name,
                };
                ConsoleEntry { row, actor }
            })
            .collect())
    }
}
