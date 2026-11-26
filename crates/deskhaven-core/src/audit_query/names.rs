//! Audit tables keep ids only (the trail outlives its accounts): a missing name is a deleted
//! account, not an error.

use std::collections::{BTreeSet, HashMap};

use deskhaven_db::entity::{api_token, user};
use sea_orm::{ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter, QuerySelect};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditUser {
    pub id: i64,
    pub username: String,
}

pub(super) async fn usernames<C: ConnectionTrait>(
    db: &C,
    ids: impl IntoIterator<Item = i64>,
) -> Result<HashMap<i64, String>, DbErr> {
    let ids: BTreeSet<i64> = ids.into_iter().collect();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = user::Entity::find()
        .select_only()
        .column(user::Column::Id)
        .column(user::Column::Username)
        .filter(user::Column::Id.is_in(ids))
        .into_tuple::<(i64, String)>()
        .all(db)
        .await?;
    Ok(rows.into_iter().collect())
}

pub(super) async fn token_names<C: ConnectionTrait>(
    db: &C,
    ids: impl IntoIterator<Item = i64>,
) -> Result<HashMap<i64, String>, DbErr> {
    let ids: BTreeSet<i64> = ids.into_iter().collect();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = api_token::Entity::find()
        .select_only()
        .column(api_token::Column::Id)
        .column(api_token::Column::Name)
        .filter(api_token::Column::Id.is_in(ids))
        .into_tuple::<(i64, String)>()
        .all(db)
        .await?;
    Ok(rows.into_iter().collect())
}

pub(super) fn user_of(names: &HashMap<i64, String>, id: Option<i64>) -> Option<AuditUser> {
    let id = id?;
    names.get(&id).map(|username| AuditUser {
        id,
        username: username.clone(),
    })
}
