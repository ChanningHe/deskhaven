use deskhaven_db::entity::user;
use sea_orm::{ColumnTrait, ConnectionTrait, DbErr, EntityTrait, PaginatorTrait, QueryFilter};

use super::normalize;

pub(crate) async fn find_by_email<C: ConnectionTrait>(
    conn: &C,
    raw: &str,
) -> Result<Option<user::Model>, DbErr> {
    let Ok(email) = normalize::email(raw) else {
        return Ok(None);
    };
    user::Entity::find()
        .filter(user::Column::EmailNorm.eq(email.norm))
        .one(conn)
        .await
}

pub(crate) async fn find_by_username<C: ConnectionTrait>(
    conn: &C,
    raw: &str,
) -> Result<Option<user::Model>, DbErr> {
    let Some(norm) = normalize::username_norm(raw) else {
        return Ok(None);
    };
    user::Entity::find()
        .filter(user::Column::UsernameNorm.eq(norm))
        .one(conn)
        .await
}

/// A malformed username counts as taken: it can never be created.
pub(crate) async fn username_taken<C: ConnectionTrait>(
    conn: &C,
    username: &str,
) -> Result<bool, DbErr> {
    let Some(norm) = normalize::username_norm(username) else {
        return Ok(true);
    };
    let found = user::Entity::find()
        .filter(user::Column::UsernameNorm.eq(norm))
        .count(conn)
        .await?;
    Ok(found > 0)
}

pub(crate) fn email_norm(raw: &str) -> Option<String> {
    normalize::email(raw).ok().map(|email| email.norm)
}

pub(crate) fn is_valid_email(raw: &str) -> bool {
    normalize::email(raw).is_ok()
}
