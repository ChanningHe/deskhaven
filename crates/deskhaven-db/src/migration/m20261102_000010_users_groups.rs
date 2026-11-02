//! Local user accounts, groups and group membership (04-data-model §3).
//!
//! Case-insensitive uniqueness uses stored normalized columns. `email_norm` is a nullable
//! UNIQUE column: SQLite and PostgreSQL both treat NULLs as distinct in unique indexes, so
//! users without an email never conflict and no partial index is needed. CHECK constraints
//! keep a buggy writer from bypassing the normalized columns.

use sea_orm::DbBackend;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{
    big_integer, big_integer_null, big_pk_auto, boolean, text, text_null,
};

use super::{bool_checks, enum_check, foreign_key, index, unique_index};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
    Username,
    UsernameNorm,
    DisplayName,
    Email,
    EmailNorm,
    EmailVerified,
    PasswordHash,
    AuthSource,
    Role,
    Status,
    MustChangePassword,
    TotpSecretEnc,
    TotpEnabledAt,
    TotpLastStep,
    Note,
    AvatarPath,
    LastLoginAt,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Groups {
    Table,
    Id,
    Name,
    Note,
    ManagedBy,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum GroupMembers {
    Table,
    GroupId,
    UserId,
    CreatedAt,
}

const AUTH_SOURCES: &[&str] = &["local", "ldap", "oidc"];
const ROLES: &[&str] = &["owner", "admin", "user"];
const STATUSES: &[&str] = &["active", "disabled", "unverified"];
const MANAGED_BY: &[&str] = &["ui", "config"];
const USERNAME_MAX_LEN: i32 = 64;

/// `[A-Za-z0-9._@-]` only (04 §3). The two backends have no common character-class syntax
/// (SQLite has no regex operator, PostgreSQL no GLOB), so the expression branches. Both reject
/// every non-ASCII character; emptiness is covered by the length check.
fn username_charset_check(backend: DbBackend) -> Check {
    let expr = if backend == DbBackend::Postgres {
        "username ~ '^[A-Za-z0-9._@-]+$'"
    } else {
        "username NOT GLOB '*[^A-Za-z0-9._@-]*'"
    };
    Check::named("ck_users_username_charset", Expr::cust(expr))
}

fn users_table(backend: DbBackend) -> TableCreateStatement {
    let username_len = || Expr::expr(Func::char_length(Expr::col(Users::Username)));
    let email_pair_consistent = Expr::col(Users::Email)
        .is_null()
        .and(Expr::col(Users::EmailNorm).is_null())
        .or(Expr::col(Users::Email)
            .is_not_null()
            .and(Expr::col(Users::EmailNorm).is_not_null()));
    let mut table = Table::create()
        .table(Users::Table)
        .col(big_pk_auto(Users::Id))
        .col(text(Users::Username))
        .col(text(Users::UsernameNorm))
        .col(text(Users::DisplayName))
        .col(text_null(Users::Email))
        .col(text_null(Users::EmailNorm))
        .col(boolean(Users::EmailVerified).default(false))
        .col(text_null(Users::PasswordHash))
        .col(text(Users::AuthSource))
        .col(text(Users::Role))
        .col(text(Users::Status))
        .col(boolean(Users::MustChangePassword).default(false))
        .col(text_null(Users::TotpSecretEnc))
        .col(big_integer_null(Users::TotpEnabledAt))
        .col(big_integer_null(Users::TotpLastStep))
        .col(text(Users::Note).default(""))
        .col(text_null(Users::AvatarPath))
        .col(big_integer_null(Users::LastLoginAt))
        .col(big_integer(Users::CreatedAt))
        .col(big_integer(Users::UpdatedAt))
        .check(enum_check(
            "ck_users_auth_source",
            Users::AuthSource,
            AUTH_SOURCES,
        ))
        .check(enum_check("ck_users_role", Users::Role, ROLES))
        .check(enum_check("ck_users_status", Users::Status, STATUSES))
        .check(Check::named(
            "ck_users_username_len",
            username_len()
                .gte(1)
                .and(username_len().lte(USERNAME_MAX_LEN)),
        ))
        .check(Check::named(
            "ck_users_username_norm",
            Expr::col(Users::UsernameNorm).eq(Func::lower(Expr::col(Users::Username))),
        ))
        .check(Check::named("ck_users_email_norm", email_pair_consistent))
        .check(Check::named(
            "ck_users_email_norm_not_empty",
            Expr::col(Users::EmailNorm).ne(""),
        ))
        .check(username_charset_check(backend))
        .check(Check::named(
            "ck_users_external_has_no_password",
            Expr::col(Users::AuthSource)
                .eq("local")
                .or(Expr::col(Users::PasswordHash).is_null()),
        ))
        .check(Check::named(
            "ck_users_totp_enabled_has_secret",
            Expr::col(Users::TotpEnabledAt)
                .is_null()
                .or(Expr::col(Users::TotpSecretEnc).is_not_null()),
        ))
        .to_owned();
    bool_checks(
        &mut table,
        backend,
        "users",
        &["email_verified", "must_change_password"],
    );
    table
}

fn groups_table() -> TableCreateStatement {
    Table::create()
        .table(Groups::Table)
        .col(big_pk_auto(Groups::Id))
        .col(text(Groups::Name))
        .col(text(Groups::Note).default(""))
        .col(text(Groups::ManagedBy).default("ui"))
        .col(big_integer(Groups::CreatedAt))
        .col(big_integer(Groups::UpdatedAt))
        .check(enum_check(
            "ck_groups_managed_by",
            Groups::ManagedBy,
            MANAGED_BY,
        ))
        .to_owned()
}

fn group_members_table() -> TableCreateStatement {
    Table::create()
        .table(GroupMembers::Table)
        .col(big_integer(GroupMembers::GroupId))
        .col(big_integer(GroupMembers::UserId))
        .col(big_integer(GroupMembers::CreatedAt))
        .primary_key(
            Index::create()
                .col(GroupMembers::GroupId)
                .col(GroupMembers::UserId),
        )
        .foreign_key(&mut foreign_key(
            "fk_group_members_group",
            (GroupMembers::Table, GroupMembers::GroupId),
            (Groups::Table, Groups::Id),
            ForeignKeyAction::Cascade,
        ))
        .foreign_key(&mut foreign_key(
            "fk_group_members_user",
            (GroupMembers::Table, GroupMembers::UserId),
            (Users::Table, Users::Id),
            ForeignKeyAction::Cascade,
        ))
        .to_owned()
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(users_table(manager.get_database_backend()))
            .await?;
        let unique_username =
            unique_index("uq_users_username_norm", Users::Table, Users::UsernameNorm);
        manager.create_index(unique_username).await?;
        let unique_email = unique_index("uq_users_email_norm", Users::Table, Users::EmailNorm);
        manager.create_index(unique_email).await?;

        manager.create_table(groups_table()).await?;
        let unique_name = unique_index("uq_groups_name", Groups::Table, Groups::Name);
        manager.create_index(unique_name).await?;

        manager.create_table(group_members_table()).await?;
        // The composite PK serves lookups by group; this one serves "groups of a user" and
        // the ON DELETE CASCADE from users.
        let by_user = index(
            "idx_group_members_user_id",
            GroupMembers::Table,
            GroupMembers::UserId,
        );
        manager.create_index(by_user).await
    }
}
