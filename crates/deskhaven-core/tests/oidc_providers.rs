//! OIDC provider management: CRUD, validation, the sealed client secret bound to its row,
//! config-managed providers, deletion with linked accounts, and the login button cache.
//! Both backends.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::sync::Arc;

use common::oidc::{Env, PUBLIC_URL};
use common::oidc_idp::{CLIENT_ID, CLIENT_SECRET};
use common::{clock, settings};
use deskhaven_core::crypto::{MasterKey, SecretBox};
use deskhaven_core::oidc::{
    LoginButton, NewProvider, OidcService, ProviderError, ProviderUpdate, ReqwestFetch,
};
use deskhaven_core::users::{PasswordHasher, UserService};
use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::entity::oidc_provider;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, EntityTrait};

async fn service(db: &TestDb, public_url: &str) -> OidcService {
    let settings = settings(db).await;
    let clock = clock();
    let users = UserService::with_clock(
        db.conn.clone(),
        settings.clone(),
        PasswordHasher::default(),
        clock.clone(),
    );
    OidcService::new(
        db.conn.clone(),
        Arc::new(SecretBox::new(&MasterKey::from_bytes(&[7; 32]))),
        settings,
        users,
        clock,
        public_url,
        Arc::new(ReqwestFetch::new()),
    )
}

fn new(name: &str) -> NewProvider {
    NewProvider::new(name, "https://idp.example.com/realms/main", CLIENT_ID)
}

#[tokio::test]
async fn create_get_list_update_delete() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db, PUBLIC_URL).await;

        let created = svc
            .create_provider(NewProvider {
                display_name: " Corp SSO ".to_owned(),
                scopes: Some("openid email groups".to_owned()),
                ..new("corp")
            })
            .await
            .unwrap();
        let updated = svc
            .update_provider(
                created.id,
                ProviderUpdate {
                    name: Some("corp2".to_owned()),
                    auto_provision: Some(true),
                    group_claim: Some(Some("groups".to_owned())),
                    ..ProviderUpdate::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(created.display_name, "Corp SSO", "{}", db.backend);
        assert_eq!(created.scopes, "openid email groups");
        assert!(!created.has_client_secret && created.enabled);
        assert_eq!(created.managed_by, ManagedBy::Ui);
        assert_eq!(updated.name, "corp2");
        assert_eq!(updated.issuer, created.issuer);
        assert!(updated.auto_provision);
        assert_eq!(updated.group_claim.as_deref(), Some("groups"));
        assert_eq!(svc.get_provider_by_name("corp2").await.unwrap(), updated);
        assert_eq!(svc.list_providers().await.unwrap(), vec![updated.clone()]);
        svc.delete_provider(created.id).await.unwrap();
        assert!(matches!(
            svc.get_provider(created.id).await,
            Err(ProviderError::NotFound)
        ));
    })
    .await;
}

#[tokio::test]
async fn invalid_input_and_duplicate_names_are_refused() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db, PUBLIC_URL).await;
        svc.create_provider(new("corp")).await.unwrap();
        let field = |r: Result<_, ProviderError>| match r {
            Err(ProviderError::Invalid { field, .. }) => field,
            other => panic!("{other:?}"),
        };

        let duplicate = svc.create_provider(new("corp")).await;
        let plain_http = svc
            .create_provider(NewProvider::new("x", "http://idp.example.com", CLIENT_ID))
            .await;
        let no_openid = svc
            .create_provider(NewProvider {
                scopes: Some("email".to_owned()),
                ..new("y")
            })
            .await;
        let bad_name = svc.create_provider(new("a b")).await;
        let no_client = svc
            .create_provider(NewProvider::new("z", "https://idp.example.com", " "))
            .await;

        assert!(
            matches!(duplicate, Err(ProviderError::Conflict)),
            "{}",
            db.backend
        );
        assert_eq!(field(plain_http), "issuer");
        assert_eq!(field(no_openid), "scopes");
        assert_eq!(field(bad_name), "name");
        assert_eq!(field(no_client), "client_id");
    })
    .await;
}

#[tokio::test]
async fn the_client_secret_is_sealed_to_its_row_and_can_be_replaced_or_removed() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db, PUBLIC_URL).await;
        let corp = svc
            .create_provider(NewProvider {
                client_secret: Some(CLIENT_SECRET.to_owned()),
                ..new("corp")
            })
            .await
            .unwrap();
        let other = svc.create_provider(new("other")).await.unwrap();
        let row = |id| async move {
            oidc_provider::Entity::find_by_id(id)
                .one(&db.conn)
                .await
                .unwrap()
                .unwrap()
        };

        let sealed = row(corp.id).await.client_secret_enc.unwrap();
        // Copied to another row, the ciphertext does not open (AAD names the row).
        let mut moved: oidc_provider::ActiveModel = row(other.id).await.into();
        moved.client_secret_enc = Set(Some(sealed.clone()));
        moved.update(&db.conn).await.unwrap();
        let removed = svc
            .update_provider(
                corp.id,
                ProviderUpdate {
                    client_secret: Some(None),
                    ..ProviderUpdate::default()
                },
            )
            .await
            .unwrap();

        assert!(corp.has_client_secret, "{}", db.backend);
        assert!(sealed.starts_with("v1:") && !sealed.contains(CLIENT_SECRET));
        let crypto = SecretBox::new(&MasterKey::from_bytes(&[7; 32]));
        let opened = crypto.open(
            &format!("oidc_providers.client_secret_enc:{}", corp.id),
            &sealed,
        );
        assert_eq!(opened.unwrap().as_slice(), CLIENT_SECRET.as_bytes());
        let elsewhere = crypto.open(
            &format!("oidc_providers.client_secret_enc:{}", other.id),
            &sealed,
        );
        assert!(elsewhere.is_err());
        assert!(!removed.has_client_secret);
        assert_eq!(row(corp.id).await.client_secret_enc, None);
    })
    .await;
}

#[tokio::test]
async fn config_managed_providers_are_read_only_and_linked_ones_cannot_be_deleted() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |p| p.auto_provision = true).await;
        env.login("sub-1", |s| s).await;
        let managed = env.oidc.create_provider(new("managed")).await.unwrap();
        let mut row: oidc_provider::ActiveModel = oidc_provider::Entity::find_by_id(managed.id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap()
            .into();
        row.managed_by = Set(ManagedBy::Config);
        row.update(&db.conn).await.unwrap();

        let in_use = env.oidc.delete_provider(env.provider.id).await;
        let update = env
            .oidc
            .update_provider(managed.id, ProviderUpdate::default())
            .await;
        let delete = env.oidc.delete_provider(managed.id).await;

        assert!(
            matches!(in_use, Err(ProviderError::InUse)),
            "{}: {in_use:?}",
            db.backend
        );
        assert!(matches!(update, Err(ProviderError::ManagedByConfig)));
        assert!(matches!(delete, Err(ProviderError::ManagedByConfig)));
    })
    .await;
}

#[tokio::test]
async fn login_buttons_list_enabled_providers_and_follow_every_write() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db, PUBLIC_URL).await;
        let unconfigured = service(db, "").await;
        let button = |name: &str, icon: Option<&str>| LoginButton {
            name: name.to_owned(),
            icon: icon.map(str::to_owned),
        };

        assert!(svc.login_buttons().await.is_empty(), "{}", db.backend);
        let corp = svc
            .create_provider(NewProvider {
                icon: Some("<svg/>".to_owned()),
                ..new("corp")
            })
            .await
            .unwrap();
        svc.create_provider(NewProvider {
            enabled: false,
            ..new("off")
        })
        .await
        .unwrap();
        assert_eq!(
            svc.login_buttons().await.to_vec(),
            vec![button("corp", Some("<svg/>"))]
        );
        svc.create_provider(new("azure")).await.unwrap();
        assert_eq!(
            svc.login_buttons().await.to_vec(),
            vec![button("azure", None), button("corp", Some("<svg/>"))]
        );
        svc.update_provider(
            corp.id,
            ProviderUpdate {
                enabled: Some(false),
                ..ProviderUpdate::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            svc.login_buttons().await.to_vec(),
            vec![button("azure", None)]
        );
        // No public_url: no login can complete, so no buttons.
        assert!(unconfigured.login_buttons().await.is_empty());
        assert!(unconfigured.redirect_uri().is_none());
    })
    .await;
}
