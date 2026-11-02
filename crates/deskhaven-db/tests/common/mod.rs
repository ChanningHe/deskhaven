//! Row builders shared by the identity & access schema tests.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used)]
// Each test crate uses a subset.
#![allow(dead_code)]

use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::entity::user::{AuthSource, Role, Status};
use deskhaven_db::entity::{
    api_token, auth_challenge, group, group_member, ldap_directory, oidc_flow, oidc_provider,
    recovery_code, session, setup_token, user, user_identity,
};
use deskhaven_db::time::{self, TimestampMs};
use sea_orm::ActiveValue::{NotSet, Set};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbErr, EntityTrait, PaginatorTrait};

/// A fixed instant with a non-zero millisecond part.
pub const T0_MS: i64 = 1_700_000_000_123;

pub fn ts(ms: i64) -> TimestampMs {
    time::from_millis(ms).unwrap()
}

pub fn user_row(username: &str) -> user::ActiveModel {
    user::ActiveModel {
        id: NotSet,
        username: Set(username.to_owned()),
        username_norm: Set(username.to_lowercase()),
        display_name: Set(username.to_owned()),
        email: Set(None),
        email_norm: Set(None),
        email_verified: Set(false),
        password_hash: Set(Some(
            "$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA".to_owned(),
        )),
        auth_source: Set(AuthSource::Local),
        role: Set(Role::User),
        status: Set(Status::Active),
        must_change_password: Set(false),
        totp_secret_enc: Set(None),
        totp_enabled_at: Set(None),
        totp_last_step: Set(None),
        note: Set(String::new()),
        avatar_path: Set(None),
        last_login_at: Set(None),
        created_at: Set(ts(T0_MS)),
        updated_at: Set(ts(T0_MS)),
    }
}

pub fn user_row_with_email(username: &str, email: &str) -> user::ActiveModel {
    user::ActiveModel {
        email: Set(Some(email.to_owned())),
        email_norm: Set(Some(email.to_lowercase())),
        ..user_row(username)
    }
}

pub fn group_row(name: &str) -> group::ActiveModel {
    group::ActiveModel {
        id: NotSet,
        name: Set(name.to_owned()),
        note: Set(String::new()),
        managed_by: Set(ManagedBy::Ui),
        created_at: Set(ts(T0_MS)),
        updated_at: Set(ts(T0_MS)),
    }
}

pub fn member_row(group_id: i64, user_id: i64) -> group_member::ActiveModel {
    group_member::ActiveModel {
        group_id: Set(group_id),
        user_id: Set(user_id),
        created_at: Set(ts(T0_MS)),
    }
}

pub fn provider_row(name: &str) -> oidc_provider::ActiveModel {
    oidc_provider::ActiveModel {
        id: NotSet,
        name: Set(name.to_owned()),
        display_name: Set(name.to_owned()),
        icon: Set(None),
        issuer: Set(format!("https://{name}.example.com")),
        client_id: Set("client".to_owned()),
        client_secret_enc: Set(Some("v1:k1:nonce:ct".to_owned())),
        scopes: Set("openid profile email".to_owned()),
        auto_provision: Set(false),
        group_claim: Set(None),
        admin_group: Set(None),
        enabled: Set(true),
        managed_by: Set(ManagedBy::Ui),
        created_at: Set(ts(T0_MS)),
        updated_at: Set(ts(T0_MS)),
    }
}

pub fn ldap_row(name: &str) -> ldap_directory::ActiveModel {
    ldap_directory::ActiveModel {
        id: NotSet,
        name: Set(name.to_owned()),
        url: Set("ldaps://ldap.example.com".to_owned()),
        start_tls: Set(false),
        tls_verify: Set(true),
        bind_dn: Set(Some("cn=svc,dc=example,dc=com".to_owned())),
        bind_password_enc: Set(Some("v1:k1:nonce:ct".to_owned())),
        user_base_dn: Set("ou=people,dc=example,dc=com".to_owned()),
        user_filter: Set("(uid={username})".to_owned()),
        username_attr: Set("uid".to_owned()),
        email_attr: Set(Some("mail".to_owned())),
        display_name_attr: Set(None),
        group_base_dn: Set(None),
        group_filter: Set(None),
        admin_group_dn: Set(None),
        enabled: Set(true),
        managed_by: Set(ManagedBy::Config),
        created_at: Set(ts(T0_MS)),
        updated_at: Set(ts(T0_MS)),
    }
}

pub fn identity_row(user_id: i64, provider_id: i64, subject: &str) -> user_identity::ActiveModel {
    user_identity::ActiveModel {
        id: NotSet,
        user_id: Set(user_id),
        provider_id: Set(provider_id),
        subject: Set(subject.to_owned()),
        email: Set(Some("a@example.com".to_owned())),
        email_verified: Set(true),
        created_at: Set(ts(T0_MS)),
    }
}

pub fn session_row(user_id: i64, token_hash: &str) -> session::ActiveModel {
    session::ActiveModel {
        id: NotSet,
        token_hash: Set(token_hash.to_owned()),
        kind: Set(session::Kind::Client),
        user_id: Set(user_id),
        rustdesk_id: Set(Some("123456789".to_owned())),
        device_uuid: Set(Some("uuid-1".to_owned())),
        device_info: Set(Some(
            r#"{"os":"linux","type":"client","name":"pc"}"#.to_owned(),
        )),
        ip: Set(Some("203.0.113.7".to_owned())),
        created_at: Set(ts(T0_MS)),
        last_used_at: Set(ts(T0_MS + 1)),
        expires_at: Set(ts(T0_MS + 3_600_000)),
        revoked_at: Set(None),
    }
}

pub fn api_token_row(created_by: Option<i64>, token_hash: &str) -> api_token::ActiveModel {
    api_token::ActiveModel {
        id: NotSet,
        name: Set("deploy".to_owned()),
        token_prefix: Set("dhk_ab12".to_owned()),
        token_hash: Set(token_hash.to_owned()),
        scopes: Set(r#"["deploy","assign"]"#.to_owned()),
        created_by: Set(created_by),
        created_at: Set(ts(T0_MS)),
        expires_at: Set(None),
        last_used_at: Set(None),
        revoked_at: Set(None),
    }
}

pub fn recovery_row(user_id: i64, code_hash: &str) -> recovery_code::ActiveModel {
    recovery_code::ActiveModel {
        id: NotSet,
        user_id: Set(user_id),
        code_hash: Set(code_hash.to_owned()),
        used_at: Set(None),
        created_at: Set(ts(T0_MS)),
    }
}

pub fn challenge_row(user_id: i64, secret_hash: &str) -> auth_challenge::ActiveModel {
    auth_challenge::ActiveModel {
        id: NotSet,
        secret_hash: Set(secret_hash.to_owned()),
        user_id: Set(user_id),
        kind: Set(auth_challenge::Kind::EmailCode),
        code_hash: Set(Some("hash".to_owned())),
        attempts: Set(0),
        context: Set(r#"{"rustdesk_id":"1"}"#.to_owned()),
        created_at: Set(ts(T0_MS)),
        expires_at: Set(ts(T0_MS + 600_000)),
        consumed_at: Set(None),
    }
}

pub fn flow_row(
    provider_id: i64,
    state_hash: &str,
    client_code_hash: Option<&str>,
) -> oidc_flow::ActiveModel {
    oidc_flow::ActiveModel {
        id: NotSet,
        state_hash: Set(state_hash.to_owned()),
        client_code_hash: Set(client_code_hash.map(str::to_owned)),
        provider_id: Set(provider_id),
        pkce_verifier_enc: Set("v1:k1:nonce:ct".to_owned()),
        nonce: Set("nonce".to_owned()),
        flow: Set(if client_code_hash.is_some() {
            oidc_flow::Flow::Client
        } else {
            oidc_flow::Flow::Console
        }),
        context: Set("{}".to_owned()),
        created_at: Set(ts(T0_MS)),
        expires_at: Set(ts(T0_MS + 600_000)),
        completed_user_id: Set(None),
        consumed_at: Set(None),
        error: Set(None),
    }
}

pub fn setup_token_row(token_hash: &str) -> setup_token::ActiveModel {
    setup_token::ActiveModel {
        id: NotSet,
        token_hash: Set(token_hash.to_owned()),
        created_at: Set(ts(T0_MS)),
        expires_at: Set(ts(T0_MS + 86_400_000)),
        used_at: Set(None),
    }
}

pub async fn count<E: EntityTrait>(db: &DatabaseConnection) -> u64
where
    E::Model: Sync,
{
    E::find().count(db).await.unwrap()
}

pub async fn exec(db: &DatabaseConnection, sql: &str) -> Result<(), DbErr> {
    db.execute_unprepared(sql).await.map(|_| ())
}
