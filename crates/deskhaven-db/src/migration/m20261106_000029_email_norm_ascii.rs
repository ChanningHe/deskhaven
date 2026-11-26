//! Unicode `to_lowercase` maps some non-ASCII characters onto ASCII letters (Kelvin sign U+212A
//! to `k`). No unique-index conflict is possible: equal ASCII lowercases had equal Unicode
//! lowercases too.

use sea_orm::{FromQueryResult, Statement};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
    Email,
    EmailNorm,
}

#[derive(FromQueryResult)]
struct Row {
    id: i64,
    email: String,
    email_norm: String,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let builder = db.get_database_backend();
        let select = Query::select()
            .columns([Users::Id, Users::Email, Users::EmailNorm])
            .from(Users::Table)
            .and_where(Expr::col(Users::Email).is_not_null())
            .to_owned();
        let rows = Row::find_by_statement(builder.build(&select))
            .all(db)
            .await?;
        for row in rows {
            let norm = row.email.to_ascii_lowercase();
            if norm == row.email_norm {
                continue;
            }
            let update = Query::update()
                .table(Users::Table)
                .value(Users::EmailNorm, norm)
                .and_where(Expr::col(Users::Id).eq(row.id))
                .to_owned();
            let stmt: Statement = builder.build(&update);
            db.execute_raw(stmt).await?;
        }
        Ok(())
    }
}
