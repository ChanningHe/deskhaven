//! Login events (`login_events`).

use deskhaven_db::entity::login_event::{Client, Column, Entity, Method, Model};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use super::listing::{Listing, text};
use super::names::{AuditUser, user_of, usernames};
use super::{AuditQuery, AuditQueryError};
use crate::pagination::Page;

/// Filters of [`AuditQuery::logins`]. `q` matches the attempted username and the address.
#[derive(Clone, Debug, Default)]
pub struct LoginQuery {
    pub listing: Listing,
    pub user_id: Option<i64>,
    pub method: Option<Method>,
    pub client: Option<Client>,
    /// The outcome label exactly (`success`, `invalid_credentials`, ...).
    pub result: Option<String>,
    /// The RustDesk id of the client device.
    pub device: Option<String>,
    pub q: Option<String>,
}

/// A login attempt with the account it matched, when that still exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginEntry {
    pub row: Model,
    pub user: Option<AuditUser>,
}

impl AuditQuery {
    pub async fn logins(&self, query: &LoginQuery) -> Result<Page<LoginEntry>, AuditQueryError> {
        let mut select = Entity::find();
        if let Some(user) = query.user_id {
            select = select.filter(Column::UserId.eq(user));
        }
        if let Some(method) = query.method {
            select = select.filter(Column::Method.eq(method));
        }
        if let Some(client) = query.client {
            select = select.filter(Column::Client.eq(client));
        }
        if let Some(result) = &query.result {
            select = select.filter(Column::Result.eq(result.as_str()));
        }
        if let Some(device) = &query.device {
            select = select.filter(Column::RustdeskId.eq(device.as_str()));
        }
        if let Some(q) = text(query.q.as_deref()) {
            let columns = [Column::UsernameAttempted, Column::Ip];
            select = select.filter(self.contains(&columns, q));
        }
        let mut page = query
            .listing
            .run(&self.db, select, (Column::CreatedAt, Column::Id))
            .await?;
        let names = usernames(&self.db, page.items.iter().filter_map(|r| r.user_id)).await?;
        let rows = std::mem::take(&mut page.items);
        let entries = rows
            .into_iter()
            .map(|row| LoginEntry {
                user: user_of(&names, row.user_id),
                row,
            })
            .collect();
        Ok(page.with_items(entries))
    }
}
