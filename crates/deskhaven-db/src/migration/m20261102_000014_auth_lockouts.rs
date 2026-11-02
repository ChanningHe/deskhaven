//! Login failure counters and lockouts (04-data-model §3 `auth_lockouts`, 06-security §3.6).
//!
//! Keyed by an opaque text key (`user:<username_norm>`), deliberately without a foreign key
//! to `users`: unknown usernames are counted exactly like existing ones, so the table never
//! tells whether an account exists. Rows are written in short read-modify-write
//! transactions by `deskhaven_core::auth::limiter`; `updated_at` is indexed for the
//! periodic cleanup of rows that no longer affect any decision. `last_success_at` marks a
//! source row as a known-good source of its username (it bypasses the global row).

use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{big_integer, big_integer_null, integer, text};

use super::index;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum AuthLockouts {
    Table,
    Key,
    Failures,
    WindowStartedAt,
    LockedUntil,
    LockLevel,
    UpdatedAt,
    LastSuccessAt,
}

fn auth_lockouts_table() -> TableCreateStatement {
    Table::create()
        .table(AuthLockouts::Table)
        .col(text(AuthLockouts::Key).primary_key())
        .col(integer(AuthLockouts::Failures))
        .col(big_integer(AuthLockouts::WindowStartedAt))
        .col(big_integer_null(AuthLockouts::LockedUntil))
        .col(integer(AuthLockouts::LockLevel))
        .col(big_integer(AuthLockouts::UpdatedAt))
        .col(big_integer_null(AuthLockouts::LastSuccessAt))
        .check(Check::named(
            "ck_auth_lockouts_failures",
            Expr::col(AuthLockouts::Failures).gte(0),
        ))
        .check(Check::named(
            "ck_auth_lockouts_lock_level",
            Expr::col(AuthLockouts::LockLevel).gte(0),
        ))
        .to_owned()
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.create_table(auth_lockouts_table()).await?;
        manager
            .create_index(index(
                "idx_auth_lockouts_updated_at",
                AuthLockouts::Table,
                AuthLockouts::UpdatedAt,
            ))
            .await
    }
}
