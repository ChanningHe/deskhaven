//! The guards decide on the committed role inside the write transaction: an admin's write
//! that got past the route (a role changed meanwhile) is refused there and nothing is
//! written, neither the change nor an audit row. Both backends.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use deskhaven_config::settings::PinSet;
use deskhaven_core::clock::SystemClock;
use deskhaven_core::console_audit::Actor;
use deskhaven_core::crypto::{MasterKey, SecretBox};
use deskhaven_core::ldap::{DirectoryError, DirectoryUpdate, NewDirectory};
use deskhaven_core::oidc::{NewProvider, ProviderError, ProviderUpdate};
use deskhaven_core::settings::SettingsService;
use deskhaven_core::state::{AppState, StateConfig};
use deskhaven_core::users::{Credential, NewUser, Password, PasswordHasher, Role};
use deskhaven_db::entity::console_audit;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{EntityTrait, PaginatorTrait};

use super::guard::{directory_guard, provider_guard};
use crate::AuditCtx;
use crate::domain_error::FORBIDDEN;

async fn state(db: &TestDb) -> AppState {
    let crypto = Arc::new(SecretBox::new(&MasterKey::from_bytes(&[9; 32])));
    let loaded = SettingsService::load(db.conn.clone(), Arc::clone(&crypto), PinSet::empty());
    let (settings, _) = loaded.await.unwrap();
    AppState::new(
        db.conn.clone(),
        settings,
        crypto,
        "test-signal-key".to_owned(),
        PasswordHasher::default(),
        Arc::new(SystemClock),
        &StateConfig::default(),
    )
}

async fn user(state: &AppState, name: &str, role: Role) -> AuditCtx {
    let credential = Credential::Local {
        password: Password::new("correct horse battery staple 42"),
    };
    let new = NewUser {
        role,
        ..NewUser::new(name, credential)
    };
    let id = state.users.create(new).await.unwrap().id;
    AuditCtx {
        actor: Actor::User(id),
        ip: IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9)),
        request_id: None,
    }
}

async fn audit_rows(db: &TestDb) -> u64 {
    console_audit::Entity::find().count(&db.conn).await.unwrap()
}

fn directory() -> NewDirectory {
    NewDirectory::new(
        "corp",
        "ldaps://ldap.example.org",
        "ou=people,dc=example,dc=org",
    )
}

#[tokio::test]
async fn an_admin_cannot_write_a_directory_even_past_the_route() {
    each_migrated_backend(async |db: &TestDb| {
        let state = state(db).await;
        let admin = user(&state, "adm", Role::Admin).await;
        let owner = user(&state, "own", Role::Owner).await;
        let id = state.ldap.create_directory(directory()).await.unwrap().id;
        let rename = || DirectoryUpdate {
            name: Some("renamed".to_owned()),
            ..DirectoryUpdate::default()
        };
        let guard = |ctx: &AuditCtx| directory_guard(&state, ctx, "ldap_directory.update");
        let refused = |r: &Result<_, DirectoryError>| {
            matches!(r, Err(DirectoryError::Refused(refused)) if refused.code == FORBIDDEN)
        };

        let mut other = directory();
        other.name = "other".to_owned();
        other.enabled = false;
        let create = state.ldap.create_directory_guarded(other, guard(&admin)).await;
        let update = state.ldap.update_directory_guarded(id, rename(), guard(&admin)).await;
        let delete = state.ldap.delete_directory_guarded(id, guard(&admin)).await;

        assert!(refused(&create), "{}: {create:?}", db.backend);
        assert!(refused(&update), "{update:?}");
        assert!(matches!(delete, Err(DirectoryError::Refused(_))), "{delete:?}");
        assert_eq!(state.ldap.list_directories().await.unwrap().len(), 1);
        assert_eq!(state.ldap.get_directory(id).await.unwrap().name, "corp");
        assert_eq!(audit_rows(db).await, 0);
        let done = state.ldap.update_directory_guarded(id, rename(), guard(&owner)).await;
        assert_eq!(done.unwrap().name, "renamed");
        assert_eq!(audit_rows(db).await, 1);
    })
    .await;
}

#[tokio::test]
async fn an_admin_cannot_write_a_provider_even_past_the_route() {
    each_migrated_backend(async |db: &TestDb| {
        let state = state(db).await;
        let admin = user(&state, "adm", Role::Admin).await;
        let owner = user(&state, "own", Role::Owner).await;
        let new = |name: &str| NewProvider::new(name, "https://idp.example.com", "deskhaven");
        let id = state.oidc.create_provider(new("corp")).await.unwrap().id;
        let disable = || ProviderUpdate {
            enabled: Some(false),
            ..ProviderUpdate::default()
        };
        let guard = |ctx: &AuditCtx| provider_guard(&state, ctx, "oidc_provider.update");
        let refused = |r: &Result<_, ProviderError>| {
            matches!(r, Err(ProviderError::Refused(refused)) if refused.code == FORBIDDEN)
        };

        let create = state.oidc.create_provider_guarded(new("other"), guard(&admin)).await;
        let update = state.oidc.update_provider_guarded(id, disable(), guard(&admin)).await;
        let delete = state.oidc.delete_provider_guarded(id, guard(&admin)).await;

        assert!(refused(&create), "{}: {create:?}", db.backend);
        assert!(refused(&update), "{update:?}");
        assert!(matches!(delete, Err(ProviderError::Refused(_))), "{delete:?}");
        assert_eq!(state.oidc.list_providers().await.unwrap().len(), 1);
        assert!(state.oidc.get_provider(id).await.unwrap().enabled);
        assert_eq!(audit_rows(db).await, 0);
        let done = state.oidc.update_provider_guarded(id, disable(), guard(&owner)).await;
        assert!(!done.unwrap().enabled);
        assert_eq!(audit_rows(db).await, 1);
    })
    .await;
}
