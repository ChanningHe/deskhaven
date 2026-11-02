//! Sessions, API tokens, recovery codes and the short-lived login/OIDC/setup state
//! (04-data-model §3, 06-security §3.2–3.4).
//!
//! Long-lived bearer secrets (`token_hash`, `code_hash`) are SHA-256 hashes; `pkce_verifier_enc`
//! is encrypted. `auth_challenges.secret`, `oidc_flows.state` and `oidc_flows.client_code` are
//! the opaque handles handed to a client for minutes, stored as given and looked up by equality.
//! Every `expires_at` is indexed for the periodic cleanup job.

use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{
    big_integer, big_integer_null, big_pk_auto, integer, text, text_null,
};

use super::{enum_check, foreign_key, index, unique_index};

#[derive(DeriveMigrationName)]
pub struct Migration;

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
enum Sessions {
    Table,
    Id,
    TokenHash,
    Kind,
    UserId,
    RustdeskId,
    DeviceUuid,
    DeviceInfo,
    Ip,
    CreatedAt,
    LastUsedAt,
    ExpiresAt,
    RevokedAt,
}

#[derive(DeriveIden)]
enum ApiTokens {
    Table,
    Id,
    Name,
    TokenPrefix,
    TokenHash,
    Scopes,
    CreatedBy,
    CreatedAt,
    ExpiresAt,
    LastUsedAt,
    RevokedAt,
}

#[derive(DeriveIden)]
enum RecoveryCodes {
    Table,
    Id,
    UserId,
    CodeHash,
    UsedAt,
    CreatedAt,
}

#[derive(DeriveIden)]
enum AuthChallenges {
    Table,
    Id,
    Secret,
    UserId,
    Kind,
    CodeHash,
    Attempts,
    Context,
    CreatedAt,
    ExpiresAt,
}

#[derive(DeriveIden)]
enum OidcFlows {
    Table,
    Id,
    State,
    ClientCode,
    ProviderId,
    PkceVerifierEnc,
    Nonce,
    Flow,
    Context,
    CreatedAt,
    ExpiresAt,
    CompletedSessionId,
    Error,
}

#[derive(DeriveIden)]
enum SetupTokens {
    Table,
    Id,
    TokenHash,
    CreatedAt,
    ExpiresAt,
    UsedAt,
}

const SESSION_KINDS: &[&str] = &["client", "console"];
const CHALLENGE_KINDS: &[&str] = &["email_code", "totp"];
const FLOW_KINDS: &[&str] = &["client", "console"];
const EMPTY_JSON_OBJECT: &str = "{}";

fn sessions_table() -> TableCreateStatement {
    Table::create()
        .table(Sessions::Table)
        .col(big_pk_auto(Sessions::Id))
        .col(text(Sessions::TokenHash))
        .col(text(Sessions::Kind))
        .col(big_integer(Sessions::UserId))
        .col(text_null(Sessions::RustdeskId))
        .col(text_null(Sessions::DeviceUuid))
        .col(text_null(Sessions::DeviceInfo))
        .col(text_null(Sessions::Ip))
        .col(big_integer(Sessions::CreatedAt))
        .col(big_integer(Sessions::LastUsedAt))
        .col(big_integer(Sessions::ExpiresAt))
        .col(big_integer_null(Sessions::RevokedAt))
        .check(enum_check(
            "ck_sessions_kind",
            Sessions::Kind,
            SESSION_KINDS,
        ))
        .foreign_key(&mut foreign_key(
            "fk_sessions_user",
            (Sessions::Table, Sessions::UserId),
            (Users::Table, Users::Id),
            ForeignKeyAction::Cascade,
        ))
        .to_owned()
}

fn api_tokens_table() -> TableCreateStatement {
    Table::create()
        .table(ApiTokens::Table)
        .col(big_pk_auto(ApiTokens::Id))
        .col(text(ApiTokens::Name))
        .col(text(ApiTokens::TokenPrefix))
        .col(text(ApiTokens::TokenHash))
        .col(text(ApiTokens::Scopes))
        .col(big_integer_null(ApiTokens::CreatedBy))
        .col(big_integer(ApiTokens::CreatedAt))
        .col(big_integer_null(ApiTokens::ExpiresAt))
        .col(big_integer_null(ApiTokens::LastUsedAt))
        .col(big_integer_null(ApiTokens::RevokedAt))
        // Tokens outlive the user who minted them (automation keeps working, audit keeps the row).
        .foreign_key(&mut foreign_key(
            "fk_api_tokens_created_by",
            (ApiTokens::Table, ApiTokens::CreatedBy),
            (Users::Table, Users::Id),
            ForeignKeyAction::SetNull,
        ))
        .to_owned()
}

fn recovery_codes_table() -> TableCreateStatement {
    Table::create()
        .table(RecoveryCodes::Table)
        .col(big_pk_auto(RecoveryCodes::Id))
        .col(big_integer(RecoveryCodes::UserId))
        .col(text(RecoveryCodes::CodeHash))
        .col(big_integer_null(RecoveryCodes::UsedAt))
        .col(big_integer(RecoveryCodes::CreatedAt))
        .foreign_key(&mut foreign_key(
            "fk_recovery_codes_user",
            (RecoveryCodes::Table, RecoveryCodes::UserId),
            (Users::Table, Users::Id),
            ForeignKeyAction::Cascade,
        ))
        .to_owned()
}

fn auth_challenges_table() -> TableCreateStatement {
    Table::create()
        .table(AuthChallenges::Table)
        .col(big_pk_auto(AuthChallenges::Id))
        .col(text(AuthChallenges::Secret))
        .col(big_integer(AuthChallenges::UserId))
        .col(text(AuthChallenges::Kind))
        .col(text_null(AuthChallenges::CodeHash))
        .col(integer(AuthChallenges::Attempts).default(0))
        .col(text(AuthChallenges::Context).default(EMPTY_JSON_OBJECT))
        .col(big_integer(AuthChallenges::CreatedAt))
        .col(big_integer(AuthChallenges::ExpiresAt))
        .check(enum_check(
            "ck_auth_challenges_kind",
            AuthChallenges::Kind,
            CHALLENGE_KINDS,
        ))
        .check(Check::named(
            "ck_auth_challenges_attempts",
            Expr::col(AuthChallenges::Attempts).gte(0),
        ))
        .foreign_key(&mut foreign_key(
            "fk_auth_challenges_user",
            (AuthChallenges::Table, AuthChallenges::UserId),
            (Users::Table, Users::Id),
            ForeignKeyAction::Cascade,
        ))
        .to_owned()
}

fn oidc_flows_table() -> TableCreateStatement {
    Table::create()
        .table(OidcFlows::Table)
        .col(big_pk_auto(OidcFlows::Id))
        .col(text(OidcFlows::State))
        .col(text_null(OidcFlows::ClientCode))
        .col(big_integer(OidcFlows::ProviderId))
        .col(text(OidcFlows::PkceVerifierEnc))
        .col(text(OidcFlows::Nonce))
        .col(text(OidcFlows::Flow))
        .col(text(OidcFlows::Context).default(EMPTY_JSON_OBJECT))
        .col(big_integer(OidcFlows::CreatedAt))
        .col(big_integer(OidcFlows::ExpiresAt))
        .col(big_integer_null(OidcFlows::CompletedSessionId))
        .col(text_null(OidcFlows::Error))
        .check(enum_check(
            "ck_oidc_flows_flow",
            OidcFlows::Flow,
            FLOW_KINDS,
        ))
        .foreign_key(&mut foreign_key(
            "fk_oidc_flows_provider",
            (OidcFlows::Table, OidcFlows::ProviderId),
            (OidcProviders::Table, OidcProviders::Id),
            ForeignKeyAction::Cascade,
        ))
        .foreign_key(&mut foreign_key(
            "fk_oidc_flows_completed_session",
            (OidcFlows::Table, OidcFlows::CompletedSessionId),
            (Sessions::Table, Sessions::Id),
            ForeignKeyAction::SetNull,
        ))
        .to_owned()
}

fn setup_tokens_table() -> TableCreateStatement {
    Table::create()
        .table(SetupTokens::Table)
        .col(big_pk_auto(SetupTokens::Id))
        .col(text(SetupTokens::TokenHash))
        .col(big_integer(SetupTokens::CreatedAt))
        .col(big_integer(SetupTokens::ExpiresAt))
        .col(big_integer_null(SetupTokens::UsedAt))
        .to_owned()
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let tables = [
            sessions_table(),
            api_tokens_table(),
            recovery_codes_table(),
            auth_challenges_table(),
            oidc_flows_table(),
            setup_tokens_table(),
        ];
        for table in tables {
            manager.create_table(table).await?;
        }

        let indexes = [
            unique_index(
                "uq_sessions_token_hash",
                Sessions::Table,
                Sessions::TokenHash,
            ),
            index("idx_sessions_user_id", Sessions::Table, Sessions::UserId),
            index(
                "idx_sessions_expires_at",
                Sessions::Table,
                Sessions::ExpiresAt,
            ),
            unique_index(
                "uq_api_tokens_token_hash",
                ApiTokens::Table,
                ApiTokens::TokenHash,
            ),
            index(
                "idx_api_tokens_created_by",
                ApiTokens::Table,
                ApiTokens::CreatedBy,
            ),
            index(
                "idx_recovery_codes_user_id",
                RecoveryCodes::Table,
                RecoveryCodes::UserId,
            ),
            unique_index(
                "uq_auth_challenges_secret",
                AuthChallenges::Table,
                AuthChallenges::Secret,
            ),
            index(
                "idx_auth_challenges_user_id",
                AuthChallenges::Table,
                AuthChallenges::UserId,
            ),
            index(
                "idx_auth_challenges_expires_at",
                AuthChallenges::Table,
                AuthChallenges::ExpiresAt,
            ),
            unique_index("uq_oidc_flows_state", OidcFlows::Table, OidcFlows::State),
            unique_index(
                "uq_oidc_flows_client_code",
                OidcFlows::Table,
                OidcFlows::ClientCode,
            ),
            index(
                "idx_oidc_flows_provider_id",
                OidcFlows::Table,
                OidcFlows::ProviderId,
            ),
            index(
                "idx_oidc_flows_completed_session_id",
                OidcFlows::Table,
                OidcFlows::CompletedSessionId,
            ),
            index(
                "idx_oidc_flows_expires_at",
                OidcFlows::Table,
                OidcFlows::ExpiresAt,
            ),
            unique_index(
                "uq_setup_tokens_token_hash",
                SetupTokens::Table,
                SetupTokens::TokenHash,
            ),
            index(
                "idx_setup_tokens_expires_at",
                SetupTokens::Table,
                SetupTokens::ExpiresAt,
            ),
        ];
        for statement in indexes {
            manager.create_index(statement).await?;
        }
        Ok(())
    }
}
