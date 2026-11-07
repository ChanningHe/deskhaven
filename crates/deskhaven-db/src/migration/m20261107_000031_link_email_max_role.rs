//! `auth.oidc.link_verified_email` (bool) becomes `auth.oidc.link_email_max_role`
//! (`off | user | admin | owner`, M4-09). A stored `true` becomes `user` (the old link only
//! ever reached `user` accounts), `false` becomes `off`; the old row is removed. An
//! unreadable old value is dropped and the new default (`user`) applies. The new row starts
//! at version 1 with a history entry saying where it came from (no history exists for the
//! new key, so the version cannot repeat). Pins under the old key are refused at startup by
//! the settings registry.

use sea_orm::FromQueryResult;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Settings {
    Table,
    Key,
    Value,
    Version,
    UpdatedBy,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum SettingsHistory {
    Table,
    Key,
    OldValue,
    NewValue,
    Version,
    UpdatedBy,
    UpdatedAt,
    Reason,
    Reset,
}

const OLD_KEY: &str = "auth.oidc.link_verified_email";
const NEW_KEY: &str = "auth.oidc.link_email_max_role";
const REASON: &str = "converted from auth.oidc.link_verified_email";

#[derive(FromQueryResult)]
struct Row {
    value: String,
    updated_by: Option<i64>,
    updated_at: i64,
}

/// The new value's stored JSON text for an old one (JSON `true` / `false`); `None` when
/// it is unreadable.
fn converted(old: &str) -> Option<&'static str> {
    match old.trim() {
        "true" => Some("\"user\""),
        "false" => Some("\"off\""),
        _ => None,
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let builder = db.get_database_backend();
        let select = Query::select()
            .columns([Settings::Value, Settings::UpdatedBy, Settings::UpdatedAt])
            .from(Settings::Table)
            .and_where(Expr::col(Settings::Key).eq(OLD_KEY))
            .to_owned();
        let Some(row) = Row::find_by_statement(builder.build(&select))
            .one(db)
            .await?
        else {
            return Ok(());
        };
        if let Some(value) = converted(&row.value) {
            db.execute_raw(builder.build(&setting(value, &row)?))
                .await?;
            db.execute_raw(builder.build(&history(value, &row)?))
                .await?;
        }
        let delete = Query::delete()
            .from_table(Settings::Table)
            .and_where(Expr::col(Settings::Key).eq(OLD_KEY))
            .to_owned();
        db.execute_raw(builder.build(&delete)).await?;
        Ok(())
    }
}

fn setting(value: &str, row: &Row) -> Result<InsertStatement, DbErr> {
    let mut insert = Query::insert();
    insert
        .into_table(Settings::Table)
        .columns([
            Settings::Key,
            Settings::Value,
            Settings::Version,
            Settings::UpdatedBy,
            Settings::UpdatedAt,
        ])
        .values([
            NEW_KEY.into(),
            value.into(),
            1i64.into(),
            row.updated_by.into(),
            row.updated_at.into(),
        ])
        .map_err(|e| DbErr::Custom(e.to_string()))?;
    Ok(insert)
}

fn history(value: &str, row: &Row) -> Result<InsertStatement, DbErr> {
    let mut insert = Query::insert();
    insert
        .into_table(SettingsHistory::Table)
        .columns([
            SettingsHistory::Key,
            SettingsHistory::OldValue,
            SettingsHistory::NewValue,
            SettingsHistory::Version,
            SettingsHistory::UpdatedBy,
            SettingsHistory::UpdatedAt,
            SettingsHistory::Reason,
            SettingsHistory::Reset,
        ])
        .values([
            NEW_KEY.into(),
            Option::<String>::None.into(),
            value.into(),
            1i64.into(),
            row.updated_by.into(),
            row.updated_at.into(),
            REASON.into(),
            false.into(),
        ])
        .map_err(|e| DbErr::Custom(e.to_string()))?;
    Ok(insert)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn booleans_convert_and_anything_else_is_dropped() {
        assert_eq!(converted("true"), Some("\"user\""));
        assert_eq!(converted(" false "), Some("\"off\""));
        assert_eq!(converted("\"yes\""), None);
        assert_eq!(converted("{"), None);
    }
}
