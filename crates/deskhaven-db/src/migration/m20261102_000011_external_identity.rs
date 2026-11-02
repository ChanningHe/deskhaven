//! External identity sources: OIDC providers, LDAP directories and the identities linking
//! users to a provider (04-data-model §3, 06-security §3.4–3.5).
//!
//! `*_secret_enc` / `*_password_enc` hold `v1:<key_id>:<nonce>:<ct>` ciphertext (06 §5).

use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{big_integer, big_pk_auto, boolean, text, text_null};

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
    Name,
    DisplayName,
    Icon,
    Issuer,
    ClientId,
    ClientSecretEnc,
    Scopes,
    AutoProvision,
    GroupClaim,
    AdminGroup,
    Enabled,
    ManagedBy,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum LdapDirectories {
    Table,
    Id,
    Name,
    Url,
    StartTls,
    TlsVerify,
    BindDn,
    BindPasswordEnc,
    UserBaseDn,
    UserFilter,
    UsernameAttr,
    EmailAttr,
    DisplayNameAttr,
    GroupBaseDn,
    GroupFilter,
    AdminGroupDn,
    Enabled,
    ManagedBy,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum UserIdentities {
    Table,
    Id,
    UserId,
    ProviderId,
    Subject,
    Email,
    EmailVerified,
    CreatedAt,
}

const MANAGED_BY: &[&str] = &["ui", "config"];
const DEFAULT_SCOPES: &str = "openid profile email";

fn oidc_providers_table() -> TableCreateStatement {
    Table::create()
        .table(OidcProviders::Table)
        .col(big_pk_auto(OidcProviders::Id))
        .col(text(OidcProviders::Name))
        .col(text(OidcProviders::DisplayName))
        .col(text_null(OidcProviders::Icon))
        .col(text(OidcProviders::Issuer))
        .col(text(OidcProviders::ClientId))
        .col(text(OidcProviders::ClientSecretEnc))
        .col(text(OidcProviders::Scopes).default(DEFAULT_SCOPES))
        .col(boolean(OidcProviders::AutoProvision).default(false))
        .col(text_null(OidcProviders::GroupClaim))
        .col(text_null(OidcProviders::AdminGroup))
        .col(boolean(OidcProviders::Enabled).default(true))
        .col(text(OidcProviders::ManagedBy).default("ui"))
        .col(big_integer(OidcProviders::CreatedAt))
        .col(big_integer(OidcProviders::UpdatedAt))
        .check(enum_check(
            "ck_oidc_providers_managed_by",
            OidcProviders::ManagedBy,
            MANAGED_BY,
        ))
        .to_owned()
}

fn ldap_directories_table() -> TableCreateStatement {
    Table::create()
        .table(LdapDirectories::Table)
        .col(big_pk_auto(LdapDirectories::Id))
        .col(text(LdapDirectories::Name))
        .col(text(LdapDirectories::Url))
        .col(boolean(LdapDirectories::StartTls).default(false))
        // Certificate verification is on unless explicitly turned off (06 §3.5).
        .col(boolean(LdapDirectories::TlsVerify).default(true))
        .col(text_null(LdapDirectories::BindDn))
        .col(text_null(LdapDirectories::BindPasswordEnc))
        .col(text(LdapDirectories::UserBaseDn))
        .col(text(LdapDirectories::UserFilter))
        .col(text(LdapDirectories::UsernameAttr))
        .col(text_null(LdapDirectories::EmailAttr))
        .col(text_null(LdapDirectories::DisplayNameAttr))
        .col(text_null(LdapDirectories::GroupBaseDn))
        .col(text_null(LdapDirectories::GroupFilter))
        .col(text_null(LdapDirectories::AdminGroupDn))
        .col(boolean(LdapDirectories::Enabled).default(true))
        .col(text(LdapDirectories::ManagedBy).default("ui"))
        .col(big_integer(LdapDirectories::CreatedAt))
        .col(big_integer(LdapDirectories::UpdatedAt))
        .check(enum_check(
            "ck_ldap_directories_managed_by",
            LdapDirectories::ManagedBy,
            MANAGED_BY,
        ))
        .to_owned()
}

fn user_identities_table() -> TableCreateStatement {
    Table::create()
        .table(UserIdentities::Table)
        .col(big_pk_auto(UserIdentities::Id))
        .col(big_integer(UserIdentities::UserId))
        .col(big_integer(UserIdentities::ProviderId))
        .col(text(UserIdentities::Subject))
        .col(text_null(UserIdentities::Email))
        .col(boolean(UserIdentities::EmailVerified).default(false))
        .col(big_integer(UserIdentities::CreatedAt))
        .foreign_key(&mut foreign_key(
            "fk_user_identities_user",
            (UserIdentities::Table, UserIdentities::UserId),
            (Users::Table, Users::Id),
            ForeignKeyAction::Cascade,
        ))
        // An identity is meaningless without its provider; the user account itself stays.
        .foreign_key(&mut foreign_key(
            "fk_user_identities_provider",
            (UserIdentities::Table, UserIdentities::ProviderId),
            (OidcProviders::Table, OidcProviders::Id),
            ForeignKeyAction::Cascade,
        ))
        .to_owned()
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.create_table(oidc_providers_table()).await?;
        let unique_name = unique_index(
            "uq_oidc_providers_name",
            OidcProviders::Table,
            OidcProviders::Name,
        );
        manager.create_index(unique_name).await?;

        manager.create_table(ldap_directories_table()).await?;
        let unique_name = unique_index(
            "uq_ldap_directories_name",
            LdapDirectories::Table,
            LdapDirectories::Name,
        );
        manager.create_index(unique_name).await?;

        manager.create_table(user_identities_table()).await?;
        // The identity key (06 §3.4): (provider, subject). Its prefix also serves the
        // provider_id FK.
        let identity_key = Index::create()
            .name("uq_user_identities_provider_subject")
            .table(UserIdentities::Table)
            .col(UserIdentities::ProviderId)
            .col(UserIdentities::Subject)
            .unique()
            .to_owned();
        manager.create_index(identity_key).await?;
        let by_user = index(
            "idx_user_identities_user_id",
            UserIdentities::Table,
            UserIdentities::UserId,
        );
        manager.create_index(by_user).await
    }
}
