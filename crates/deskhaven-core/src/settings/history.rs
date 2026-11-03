//! Read side of `settings_history`: the changes of one setting, newest first.

use std::collections::HashMap;

use deskhaven_config::settings::{Problem, lookup};
use deskhaven_db::entity::{setting_history, user};
use sea_orm::{
    ColumnTrait, DbErr, EntityTrait, Order, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
};
use serde_json::{Value as Json, json};

use super::{SettingsError, SettingsService};
use crate::pagination::{Page, PageRequest};

/// One change of a setting. Values are public JSON; a sensitive setting's are always
/// `{"set": bool}`, whatever the stored text says.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryEntry {
    pub version: i64,
    /// `None`: the key had no stored value before; `Some(null)`: it had an unusable one.
    pub old_value: Option<Json>,
    /// The value after the change; for a reset, the default that applies again.
    pub new_value: Json,
    pub reset: bool,
    pub updated_by: Option<i64>,
    /// The author's username, when the account still exists.
    pub updated_by_username: Option<String>,
    /// Unix milliseconds.
    pub updated_at: i64,
    pub reason: Option<String>,
}

impl SettingsService {
    /// Usernames of those of `ids` that still exist, for "last changed by".
    pub async fn authors(
        &self,
        ids: impl IntoIterator<Item = i64>,
    ) -> Result<HashMap<i64, String>, SettingsError> {
        Ok(usernames(&self.inner.db, ids).await?)
    }

    /// One page of the history of `key`, newest first. An unknown key is
    /// [`SettingsError::Invalid`].
    pub async fn history(
        &self,
        key: &str,
        page: PageRequest,
    ) -> Result<Page<HistoryEntry>, SettingsError> {
        let setting = lookup(key).ok_or_else(|| invalid("key", "unknown setting"))?;
        let index = page.index().map_err(|p| invalid(p.field, &p.reason))?;
        let paginator = setting_history::Entity::find()
            .filter(setting_history::Column::Key.eq(setting.key()))
            .order_by(setting_history::Column::Id, Order::Desc)
            .paginate(&self.inner.db, page.page_size);
        let total = paginator.num_items().await?;
        let rows = paginator.fetch_page(index).await?;
        let names = usernames(&self.inner.db, rows.iter().filter_map(|r| r.updated_by)).await?;
        let sensitive = setting.is_sensitive();
        let items = rows
            .into_iter()
            .map(|row| entry(row, sensitive, &names))
            .collect();
        Ok(Page {
            items,
            total,
            page: page.page,
            page_size: page.page_size,
        })
    }
}

fn invalid(field: &str, reason: &str) -> SettingsError {
    SettingsError::Invalid {
        problems: vec![Problem {
            key: field.to_owned(),
            reason: reason.to_owned(),
        }],
    }
}

async fn usernames(
    db: &sea_orm::DatabaseConnection,
    ids: impl IntoIterator<Item = i64>,
) -> Result<HashMap<i64, String>, DbErr> {
    let mut ids: Vec<i64> = ids.into_iter().collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let found = user::Entity::find()
        .select_only()
        .columns([user::Column::Id, user::Column::Username])
        .filter(user::Column::Id.is_in(ids))
        .into_tuple::<(i64, String)>()
        .all(db)
        .await?;
    Ok(found.into_iter().collect())
}

fn entry(
    row: setting_history::Model,
    sensitive: bool,
    names: &HashMap<i64, String>,
) -> HistoryEntry {
    let show = |text: &str| {
        let value = serde_json::from_str(text).unwrap_or(Json::Null);
        match sensitive {
            true => mask(&value),
            false => value,
        }
    };
    HistoryEntry {
        version: row.version,
        old_value: row.old_value.as_deref().map(show),
        new_value: show(&row.new_value),
        reset: row.reset,
        updated_by: row.updated_by,
        updated_by_username: row.updated_by.and_then(|id| names.get(&id).cloned()),
        updated_at: row.updated_at.timestamp_millis(),
        reason: row.reason,
    }
}

/// `{"set": bool}` for whatever a sensitive setting's history holds: only the marker
/// survives, anything else (it should not exist) counts as unset.
fn mask(value: &Json) -> Json {
    match value.get("set").and_then(Json::as_bool) {
        Some(set) => json!({ "set": set }),
        None if value.is_null() => Json::Null,
        None => json!({ "set": false }),
    }
}
