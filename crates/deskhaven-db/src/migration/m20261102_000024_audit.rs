//! Audit tables (04-data-model §7): connection sessions, file transfers, alarms, audit nonces,
//! login events and the console audit log.
//!
//! The audit trail must outlive what it describes, so devices are referenced by their RustDesk
//! id as plain text (no foreign key to `devices`) and `console_audit.actor_id` has none to
//! users or tokens. Where a user is referenced and meaningful (`connection_sessions.
//! controller_user_id`, `login_events.user_id`) the foreign key is `ON DELETE SET NULL`.
//! A connection is reported in up to three messages that may arrive in any order, and the
//! first one creates the row: apart from its identity (device, `conn_id`, `session_id`, guid)
//! every `connection_sessions` column is optional, and `created_at` (first message received)
//! is added because `started_at` may be missing. Messages are de-duplicated by
//! `audit_nonces`, which is pruned by `received_at`. Every `created_at` / `received_at` is
//! indexed for retention pruning and listing.

use sea_orm::DbBackend;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{
    big_integer, big_integer_null, big_pk_auto, boolean, integer, integer_null, text, text_null,
};

use super::{bool_checks, enum_check, foreign_key, index, unique_index};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum ConnectionSessions {
    Table,
    Id,
    Guid,
    DeviceRustdeskId,
    ConnId,
    SessionId,
    PeerRustdeskId,
    PeerName,
    ControllerUserId,
    ConnType,
    PrimaryAuth,
    TwoFactor,
    Ip,
    StartedAt,
    AuthorizedAt,
    EndedAt,
    Note,
    ControllerNote,
    CreatedAt,
}

#[derive(DeriveIden)]
enum FileTransfers {
    Table,
    Id,
    DeviceRustdeskId,
    PeerRustdeskId,
    ConnId,
    Direction,
    Path,
    IsFile,
    FileCount,
    Files,
    Ip,
    CreatedAt,
}

#[derive(DeriveIden)]
enum Alarms {
    Table,
    Id,
    DeviceRustdeskId,
    Typ,
    Info,
    ConnId,
    CreatedAt,
}

#[derive(DeriveIden)]
enum AuditNonces {
    Table,
    Nonce,
    ReceivedAt,
}

#[derive(DeriveIden)]
enum LoginEvents {
    Table,
    Id,
    UserId,
    UsernameAttempted,
    Method,
    Result,
    Client,
    RustdeskId,
    Ip,
    CreatedAt,
}

#[derive(DeriveIden)]
enum ConsoleAudit {
    Table,
    Id,
    ActorKind,
    ActorId,
    Action,
    TargetType,
    TargetId,
    Before,
    After,
    Ip,
    CreatedAt,
}

const DIRECTIONS: &[&str] = &["send", "receive"];
const LOGIN_METHODS: &[&str] = &["password", "totp", "email_code", "oidc", "ldap", "recovery"];
const LOGIN_CLIENTS: &[&str] = &["rustdesk", "console", "api"];
const ACTOR_KINDS: &[&str] = &["user", "api_token", "cli", "system"];
const EMPTY_JSON_OBJECT: &str = "{}";
const EMPTY_JSON_ARRAY: &str = "[]";

fn connection_sessions_table() -> TableCreateStatement {
    Table::create()
        .table(ConnectionSessions::Table)
        .col(big_pk_auto(ConnectionSessions::Id))
        .col(text(ConnectionSessions::Guid))
        .col(text(ConnectionSessions::DeviceRustdeskId))
        .col(big_integer(ConnectionSessions::ConnId))
        .col(text(ConnectionSessions::SessionId))
        .col(text_null(ConnectionSessions::PeerRustdeskId))
        .col(text_null(ConnectionSessions::PeerName))
        .col(big_integer_null(ConnectionSessions::ControllerUserId))
        // 0 remote control .. 4 terminal today; no CHECK, so a newer client's value is stored.
        .col(integer_null(ConnectionSessions::ConnType))
        .col(integer_null(ConnectionSessions::PrimaryAuth))
        .col(integer_null(ConnectionSessions::TwoFactor))
        .col(text_null(ConnectionSessions::Ip))
        .col(big_integer_null(ConnectionSessions::StartedAt))
        .col(big_integer_null(ConnectionSessions::AuthorizedAt))
        .col(big_integer_null(ConnectionSessions::EndedAt))
        .col(text_null(ConnectionSessions::Note))
        .col(text_null(ConnectionSessions::ControllerNote))
        .col(big_integer(ConnectionSessions::CreatedAt))
        .foreign_key(&mut foreign_key(
            "fk_connection_sessions_controller",
            (
                ConnectionSessions::Table,
                ConnectionSessions::ControllerUserId,
            ),
            (Users::Table, Users::Id),
            ForeignKeyAction::SetNull,
        ))
        .to_owned()
}

fn file_transfers_table(backend: DbBackend) -> TableCreateStatement {
    let mut table = Table::create()
        .table(FileTransfers::Table)
        .col(big_pk_auto(FileTransfers::Id))
        .col(text(FileTransfers::DeviceRustdeskId))
        .col(text(FileTransfers::PeerRustdeskId))
        .col(big_integer(FileTransfers::ConnId))
        .col(text(FileTransfers::Direction))
        .col(text(FileTransfers::Path))
        .col(boolean(FileTransfers::IsFile))
        .col(integer(FileTransfers::FileCount))
        .col(text(FileTransfers::Files).default(EMPTY_JSON_ARRAY))
        .col(text_null(FileTransfers::Ip))
        .col(big_integer(FileTransfers::CreatedAt))
        .check(enum_check(
            "ck_file_transfers_direction",
            FileTransfers::Direction,
            DIRECTIONS,
        ))
        .check(Check::named(
            "ck_file_transfers_file_count",
            Expr::col(FileTransfers::FileCount).gte(0),
        ))
        .to_owned();
    bool_checks(&mut table, backend, "file_transfers", &["is_file"]);
    table
}

fn alarms_table() -> TableCreateStatement {
    Table::create()
        .table(Alarms::Table)
        .col(big_pk_auto(Alarms::Id))
        .col(text(Alarms::DeviceRustdeskId))
        .col(integer(Alarms::Typ))
        .col(text(Alarms::Info).default(EMPTY_JSON_OBJECT))
        .col(big_integer(Alarms::ConnId))
        .col(big_integer(Alarms::CreatedAt))
        .to_owned()
}

fn audit_nonces_table() -> TableCreateStatement {
    Table::create()
        .table(AuditNonces::Table)
        .col(text(AuditNonces::Nonce).primary_key())
        .col(big_integer(AuditNonces::ReceivedAt))
        .to_owned()
}

fn login_events_table() -> TableCreateStatement {
    Table::create()
        .table(LoginEvents::Table)
        .col(big_pk_auto(LoginEvents::Id))
        .col(big_integer_null(LoginEvents::UserId))
        .col(text(LoginEvents::UsernameAttempted))
        .col(text(LoginEvents::Method))
        .col(text(LoginEvents::Result))
        .col(text(LoginEvents::Client))
        .col(text_null(LoginEvents::RustdeskId))
        .col(text_null(LoginEvents::Ip))
        .col(big_integer(LoginEvents::CreatedAt))
        .check(enum_check(
            "ck_login_events_method",
            LoginEvents::Method,
            LOGIN_METHODS,
        ))
        .check(enum_check(
            "ck_login_events_client",
            LoginEvents::Client,
            LOGIN_CLIENTS,
        ))
        .foreign_key(&mut foreign_key(
            "fk_login_events_user",
            (LoginEvents::Table, LoginEvents::UserId),
            (Users::Table, Users::Id),
            ForeignKeyAction::SetNull,
        ))
        .to_owned()
}

fn console_audit_table() -> TableCreateStatement {
    Table::create()
        .table(ConsoleAudit::Table)
        .col(big_pk_auto(ConsoleAudit::Id))
        .col(text(ConsoleAudit::ActorKind))
        .col(big_integer_null(ConsoleAudit::ActorId))
        .col(text(ConsoleAudit::Action))
        .col(text(ConsoleAudit::TargetType))
        .col(text(ConsoleAudit::TargetId))
        .col(text_null(ConsoleAudit::Before))
        .col(text_null(ConsoleAudit::After))
        .col(text_null(ConsoleAudit::Ip))
        .col(big_integer(ConsoleAudit::CreatedAt))
        .check(enum_check(
            "ck_console_audit_actor_kind",
            ConsoleAudit::ActorKind,
            ACTOR_KINDS,
        ))
        .to_owned()
}

fn indexes() -> Vec<IndexCreateStatement> {
    vec![
        unique_index(
            "uq_connection_sessions_guid",
            ConnectionSessions::Table,
            ConnectionSessions::Guid,
        ),
        // The three messages of one connection merge into one row through this key. The
        // controller's note message and `conn/active` look a connection up by device and
        // session only, so `conn_id` comes last and the key serves those lookups too.
        Index::create()
            .name("uq_connection_sessions_conn")
            .table(ConnectionSessions::Table)
            .col(ConnectionSessions::DeviceRustdeskId)
            .col(ConnectionSessions::SessionId)
            .col(ConnectionSessions::ConnId)
            .unique()
            .to_owned(),
        // A device's connections, newest first.
        Index::create()
            .name("idx_connection_sessions_device_created")
            .table(ConnectionSessions::Table)
            .col(ConnectionSessions::DeviceRustdeskId)
            .col(ConnectionSessions::CreatedAt)
            .to_owned(),
        index(
            "idx_connection_sessions_controller_user_id",
            ConnectionSessions::Table,
            ConnectionSessions::ControllerUserId,
        ),
        index(
            "idx_connection_sessions_created_at",
            ConnectionSessions::Table,
            ConnectionSessions::CreatedAt,
        ),
        index(
            "idx_file_transfers_device_rustdesk_id",
            FileTransfers::Table,
            FileTransfers::DeviceRustdeskId,
        ),
        index(
            "idx_file_transfers_created_at",
            FileTransfers::Table,
            FileTransfers::CreatedAt,
        ),
        index(
            "idx_alarms_device_rustdesk_id",
            Alarms::Table,
            Alarms::DeviceRustdeskId,
        ),
        index("idx_alarms_created_at", Alarms::Table, Alarms::CreatedAt),
        index(
            "idx_audit_nonces_received_at",
            AuditNonces::Table,
            AuditNonces::ReceivedAt,
        ),
        // A user's history, newest first; the leading column also serves the SET NULL.
        Index::create()
            .name("idx_login_events_user_created")
            .table(LoginEvents::Table)
            .col(LoginEvents::UserId)
            .col(LoginEvents::CreatedAt)
            .to_owned(),
        index(
            "idx_login_events_created_at",
            LoginEvents::Table,
            LoginEvents::CreatedAt,
        ),
        index(
            "idx_console_audit_created_at",
            ConsoleAudit::Table,
            ConsoleAudit::CreatedAt,
        ),
    ]
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let tables = [
            connection_sessions_table(),
            file_transfers_table(backend),
            alarms_table(),
            audit_nonces_table(),
            login_events_table(),
            console_audit_table(),
        ];
        for table in tables {
            manager.create_table(table).await?;
        }
        for statement in indexes() {
            manager.create_index(statement).await?;
        }
        Ok(())
    }
}
