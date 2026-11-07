//! Explicit OIDC linking (M4-09):
//!
//! - `user_identities.linked_via` (`provisioned | email | explicit`): how the identity came
//!   to its account. Logins through an `email` identity re-check the email-link conditions
//!   every time; the others do not. Existing rows: the first identity of each `oidc`
//!   account was created with it (provisioning); every other identity was linked by
//!   email, the only other way before this migration. When in doubt `email`: the strict
//!   reading, it is re-checked.
//! - `oidc_flows.flow` admits `link`: a signed-in user linking an identity to their own
//!   account. PostgreSQL swaps the CHECK in place. SQLite cannot alter a CHECK, so the
//!   table is rebuilt (create the new shape, copy the rows, drop the old table, rename,
//!   re-create the indexes). Nothing references `oidc_flows`.

use sea_orm::DbBackend;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{big_integer, big_integer_null, big_pk_auto, text, text_null};

use super::{enum_check, foreign_key, index, unique_index};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum UserIdentities {
    Table,
    LinkedVia,
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum OidcProviders {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum OidcFlows {
    Table,
    Id,
    StateHash,
    ClientCodeHash,
    ProviderId,
    PkceVerifierEnc,
    Nonce,
    Flow,
    Context,
    CreatedAt,
    ExpiresAt,
    CompletedUserId,
    ConsumedAt,
    Error,
}

const LINK_SOURCES: &[&str] = &["provisioned", "email", "explicit"];
const FLOW_KINDS: &[&str] = &["client", "console", "link"];
const REBUILT: &str = "oidc_flows_m30";

const MARK_PROVISIONED: &str = "UPDATE user_identities SET linked_via = 'provisioned' \
     WHERE id IN (SELECT MIN(i.id) FROM user_identities i \
     JOIN users u ON u.id = i.user_id WHERE u.auth_source = 'oidc' GROUP BY i.user_id)";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        add_linked_via(manager).await?;
        match manager.get_database_backend() {
            DbBackend::Sqlite => rebuild_flows_sqlite(manager).await,
            _ => swap_flow_check_postgres(manager).await,
        }
    }
}

async fn add_linked_via(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let check = Expr::col(UserIdentities::LinkedVia).is_in(LINK_SOURCES.iter().copied());
    let add = Table::alter()
        .table(UserIdentities::Table)
        .add_column(
            text(UserIdentities::LinkedVia)
                .default("email")
                .check(check),
        )
        .to_owned();
    manager.alter_table(add).await?;
    manager
        .get_connection()
        .execute_unprepared(MARK_PROVISIONED)
        .await?;
    Ok(())
}

async fn swap_flow_check_postgres(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let kinds = FLOW_KINDS
        .iter()
        .map(|k| format!("'{k}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "ALTER TABLE oidc_flows DROP CONSTRAINT ck_oidc_flows_flow, \
         ADD CONSTRAINT ck_oidc_flows_flow CHECK (flow IN ({kinds}))"
    );
    manager.get_connection().execute_unprepared(&sql).await?;
    Ok(())
}

async fn rebuild_flows_sqlite(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let rebuilt = Alias::new(REBUILT).into_iden();
    manager.create_table(flows_table(rebuilt.clone())).await?;
    let copy = format!("INSERT INTO {REBUILT} SELECT * FROM oidc_flows");
    manager.get_connection().execute_unprepared(&copy).await?;
    manager
        .drop_table(Table::drop().table(OidcFlows::Table).to_owned())
        .await?;
    manager
        .rename_table(Table::rename().table(rebuilt, OidcFlows::Table).to_owned())
        .await?;
    for index in flow_indexes() {
        manager.create_index(index).await?;
    }
    Ok(())
}

/// `oidc_flows` as migration 000012 created it (same column order: the copy is
/// `SELECT *`), with `link` among the flow kinds.
fn flows_table(name: DynIden) -> TableCreateStatement {
    Table::create()
        .table(name.clone())
        .col(big_pk_auto(OidcFlows::Id))
        .col(text(OidcFlows::StateHash))
        .col(text_null(OidcFlows::ClientCodeHash))
        .col(big_integer(OidcFlows::ProviderId))
        .col(text(OidcFlows::PkceVerifierEnc))
        .col(text(OidcFlows::Nonce))
        .col(text(OidcFlows::Flow))
        .col(text(OidcFlows::Context).default("{}"))
        .col(big_integer(OidcFlows::CreatedAt))
        .col(big_integer(OidcFlows::ExpiresAt))
        .col(big_integer_null(OidcFlows::CompletedUserId))
        .col(big_integer_null(OidcFlows::ConsumedAt))
        .col(text_null(OidcFlows::Error))
        .check(enum_check(
            "ck_oidc_flows_flow",
            OidcFlows::Flow,
            FLOW_KINDS,
        ))
        .check(Check::named(
            "ck_oidc_flows_client_code_hash",
            client_code_exactly_for_client_flows(),
        ))
        .foreign_key(&mut foreign_key(
            "fk_oidc_flows_provider",
            (name.clone(), OidcFlows::ProviderId.into_iden()),
            (
                OidcProviders::Table.into_iden(),
                OidcProviders::Id.into_iden(),
            ),
            ForeignKeyAction::Cascade,
        ))
        .foreign_key(&mut foreign_key(
            "fk_oidc_flows_completed_user",
            (name, OidcFlows::CompletedUserId.into_iden()),
            (Users::Table.into_iden(), Users::Id.into_iden()),
            ForeignKeyAction::Cascade,
        ))
        .to_owned()
}

fn client_code_exactly_for_client_flows() -> Expr {
    Expr::col(OidcFlows::Flow)
        .eq("client")
        .and(Expr::col(OidcFlows::ClientCodeHash).is_not_null())
        .or(Expr::col(OidcFlows::Flow)
            .ne("client")
            .and(Expr::col(OidcFlows::ClientCodeHash).is_null()))
}

fn flow_indexes() -> [IndexCreateStatement; 5] {
    [
        unique_index(
            "uq_oidc_flows_state_hash",
            OidcFlows::Table,
            OidcFlows::StateHash,
        ),
        unique_index(
            "uq_oidc_flows_client_code_hash",
            OidcFlows::Table,
            OidcFlows::ClientCodeHash,
        ),
        index(
            "idx_oidc_flows_provider_id",
            OidcFlows::Table,
            OidcFlows::ProviderId,
        ),
        index(
            "idx_oidc_flows_completed_user_id",
            OidcFlows::Table,
            OidcFlows::CompletedUserId,
        ),
        index(
            "idx_oidc_flows_expires_at",
            OidcFlows::Table,
            OidcFlows::ExpiresAt,
        ),
    ]
}
