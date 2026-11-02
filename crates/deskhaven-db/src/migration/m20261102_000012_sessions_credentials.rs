//! Sessions, API tokens, recovery codes and the short-lived login/OIDC/setup state
//! (04-data-model §3, 06-security §3.2–3.4).
//!
//! Every bearer value is stored hashed: `sessions.token_hash`, `api_tokens.token_hash`,
//! `setup_tokens.token_hash`, `auth_challenges.secret_hash`, `oidc_flows.state_hash` and
//! `oidc_flows.client_code_hash` are SHA-256 of 256-bit random values (looked up by equality).
//! `auth_challenges.code_hash` and `recovery_codes.code_hash` hold low-entropy secrets and are
//! HMACs computed by the service; the columns are plain TEXT. `pkce_verifier_enc` is encrypted.
//! Challenges and OIDC flows are single-use (`consumed_at`, conditional UPDATE). Every
//! `expires_at` is indexed for the periodic cleanup job.

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
    SecretHash,
    UserId,
    Kind,
    CodeHash,
    Attempts,
    Context,
    CreatedAt,
    ExpiresAt,
    ConsumedAt,
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

/// `(col = value) = (other IS NOT NULL)`, spelled out with AND/OR so it reads the same on both
/// backends.
fn present_exactly_when(col: impl IntoIden, value: &str, other: impl IntoIden) -> Expr {
    let (col, other) = (col.into_iden(), other.into_iden());
    Expr::col(col.clone())
        .eq(value)
        .and(Expr::col(other.clone()).is_not_null())
        .or(Expr::col(col).ne(value).and(Expr::col(other).is_null()))
}

fn sessions_table() -> TableCreateStatement {
    // Client sessions are bound to a device (06 §3.2); console sessions are not.
    let device_bound = Expr::col(Sessions::Kind)
        .eq("console")
        .or(Expr::col(Sessions::RustdeskId)
            .is_not_null()
            .and(Expr::col(Sessions::DeviceUuid).is_not_null()));
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
        .check(Check::named(
            "ck_sessions_client_device_bound",
            device_bound,
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
        // A token carries its creator's authority, so it dies with them (06 §3.2).
        .foreign_key(&mut foreign_key(
            "fk_api_tokens_created_by",
            (ApiTokens::Table, ApiTokens::CreatedBy),
            (Users::Table, Users::Id),
            ForeignKeyAction::Cascade,
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
        .col(text(AuthChallenges::SecretHash))
        .col(big_integer(AuthChallenges::UserId))
        .col(text(AuthChallenges::Kind))
        .col(text_null(AuthChallenges::CodeHash))
        .col(integer(AuthChallenges::Attempts).default(0))
        .col(text(AuthChallenges::Context).default(EMPTY_JSON_OBJECT))
        .col(big_integer(AuthChallenges::CreatedAt))
        .col(big_integer(AuthChallenges::ExpiresAt))
        .col(big_integer_null(AuthChallenges::ConsumedAt))
        .check(enum_check(
            "ck_auth_challenges_kind",
            AuthChallenges::Kind,
            CHALLENGE_KINDS,
        ))
        .check(Check::named(
            "ck_auth_challenges_attempts",
            Expr::col(AuthChallenges::Attempts).gte(0),
        ))
        // An emailed code exists exactly for email_code challenges.
        .check(Check::named(
            "ck_auth_challenges_code_hash",
            present_exactly_when(AuthChallenges::Kind, "email_code", AuthChallenges::CodeHash),
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
        .col(text(OidcFlows::StateHash))
        .col(text_null(OidcFlows::ClientCodeHash))
        .col(big_integer(OidcFlows::ProviderId))
        .col(text(OidcFlows::PkceVerifierEnc))
        .col(text(OidcFlows::Nonce))
        .col(text(OidcFlows::Flow))
        .col(text(OidcFlows::Context).default(EMPTY_JSON_OBJECT))
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
        // Only client flows are polled with a client code.
        .check(Check::named(
            "ck_oidc_flows_client_code_hash",
            present_exactly_when(OidcFlows::Flow, "client", OidcFlows::ClientCodeHash),
        ))
        .foreign_key(&mut foreign_key(
            "fk_oidc_flows_provider",
            (OidcFlows::Table, OidcFlows::ProviderId),
            (OidcProviders::Table, OidcProviders::Id),
            ForeignKeyAction::Cascade,
        ))
        // The session is minted when the flow is consumed, so only the user is recorded.
        .foreign_key(&mut foreign_key(
            "fk_oidc_flows_completed_user",
            (OidcFlows::Table, OidcFlows::CompletedUserId),
            (Users::Table, Users::Id),
            ForeignKeyAction::Cascade,
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

fn indexes() -> Vec<IndexCreateStatement> {
    vec![
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
        // Idle-expiry sweeps.
        index(
            "idx_sessions_last_used_at",
            Sessions::Table,
            Sessions::LastUsedAt,
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
        // Also serves lookups by user_id (leading column) and the user-delete cascade.
        Index::create()
            .name("uq_recovery_codes_user_code")
            .table(RecoveryCodes::Table)
            .col(RecoveryCodes::UserId)
            .col(RecoveryCodes::CodeHash)
            .unique()
            .to_owned(),
        unique_index(
            "uq_auth_challenges_secret_hash",
            AuthChallenges::Table,
            AuthChallenges::SecretHash,
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
    ]
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
        for statement in indexes() {
            manager.create_index(statement).await?;
        }
        Ok(())
    }
}
