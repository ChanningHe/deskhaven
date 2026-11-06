//! Secret catalog options at rest (06 §5): sealed per value under
//! `strategies.options:<id>:<key>`, opened for the console and the heartbeat, bound to their
//! row and key, and plaintext rows from before sealing are sealed by
//! `seal_plaintext_secrets`.

#![allow(clippy::unwrap_used, clippy::panic)]

use deskhaven_core::strategies::{NewStrategy, Options, StrategyUpdate, Target};
use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::entity::strategy;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait};

use super::support::{Ctx, ctx, device_inputs, opts};

const PASSWORD: &str = "hunter2-proxy";

/// The stored options of `id`, as the database holds them.
async fn stored(db: &TestDb, id: i64) -> Options {
    let row = strategy::Entity::find_by_id(id)
        .one(&db.conn)
        .await
        .unwrap()
        .unwrap();
    serde_json::from_str(&row.options).unwrap()
}

async fn write_stored(db: &TestDb, id: i64, options: &Options) {
    let row = strategy::ActiveModel {
        id: Set(id),
        options: Set(serde_json::to_string(options).unwrap()),
        ..Default::default()
    };
    row.update(&db.conn).await.unwrap();
}

/// A row written before secrets were sealed: every value in clear.
async fn insert_plaintext(db: &TestDb, name: &str, options: &Options) -> i64 {
    let now = time::now();
    let row = strategy::ActiveModel {
        name: Set(name.to_owned()),
        options: Set(serde_json::to_string(options).unwrap()),
        note: Set(String::new()),
        managed_by: Set(ManagedBy::Ui),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    };
    row.insert(&db.conn).await.unwrap().id
}

async fn effective_global(c: &Ctx, id: i64) -> Options {
    c.svc.assign(Target::Global, id).await.unwrap();
    c.options(&device_inputs(1, None, None)).await
}

#[tokio::test]
async fn secret_options_are_sealed_at_rest_and_opened_for_the_console_and_devices() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let pairs = [
            ("proxy-password", PASSWORD),
            ("preset-address-book-password", "book-pw"),
            ("proxy-username", "bob"),
            ("ice-servers", ""),
        ];
        let created = c
            .svc
            .create(NewStrategy::new("p", opts(&pairs)))
            .await
            .unwrap();

        let raw = stored(db, created.id).await;
        let b = db.backend;
        assert!(raw["proxy-password"].starts_with("v1:"), "{b}: {raw:?}");
        assert!(
            raw["preset-address-book-password"].starts_with("v1:"),
            "{b}"
        );
        assert!(
            !serde_json::to_string(&raw).unwrap().contains(PASSWORD),
            "{b}"
        );
        // Not secret, or empty (the client deletes the option): kept in clear.
        assert_eq!(raw["proxy-username"], "bob", "{b}");
        assert_eq!(raw["ice-servers"], "", "{b}");
        assert_eq!(created.options, opts(&pairs), "{b}");
        assert_eq!(
            c.svc.get(created.id).await.unwrap().options,
            opts(&pairs),
            "{b}"
        );
        assert_eq!(effective_global(&c, created.id).await, opts(&pairs), "{b}");
    })
    .await;
}

#[tokio::test]
async fn a_patch_reseals_the_kept_secret_and_never_stores_it_in_clear() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let created = c.strategy("p", &[("proxy-password", PASSWORD)]).await;
        let before = stored(db, created.id).await;

        let patch = StrategyUpdate {
            options_patch: [("proxy-username".to_owned(), Some("bob".to_owned()))].into(),
            ..StrategyUpdate::default()
        };
        let updated = c.svc.update(created.id, patch).await.unwrap();

        let after = stored(db, created.id).await;
        let b = db.backend;
        assert!(after["proxy-password"].starts_with("v1:"), "{b}");
        assert_ne!(
            after["proxy-password"], before["proxy-password"],
            "{b}: fresh nonce"
        );
        let expected = opts(&[("proxy-password", PASSWORD), ("proxy-username", "bob")]);
        assert_eq!(updated.options, expected, "{b}");
    })
    .await;
}

#[tokio::test]
async fn a_sealed_value_moved_to_another_row_or_key_does_not_open() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let source = c.strategy("a", &[("proxy-password", PASSWORD)]).await;
        let sealed = stored(db, source.id).await["proxy-password"].clone();
        let other_row = c.strategy("b", &[]).await;
        let other_key = c.strategy("c", &[]).await;
        let moved = |key: &str| Options::from([(key.to_owned(), sealed.clone())]);
        write_stored(db, other_row.id, &moved("proxy-password")).await;
        write_stored(db, other_key.id, &moved("preset-address-book-password")).await;

        let b = db.backend;
        for id in [other_row.id, other_key.id] {
            let got = c.svc.get(id).await.unwrap();
            assert!(got.options_corrupt && got.options.is_empty(), "{b}: {id}");
        }
        assert!(effective_global(&c, other_row.id).await.is_empty(), "{b}");
        assert!(!c.svc.get(source.id).await.unwrap().options_corrupt, "{b}");
    })
    .await;
}

#[tokio::test]
async fn plaintext_secrets_of_older_rows_are_served_then_sealed_in_place() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let legacy = opts(&[("proxy-password", PASSWORD), ("k", "v")]);
        let id = insert_plaintext(db, "old", &legacy).await;
        let clean = c.strategy("new", &[("proxy-password", "x")]).await;
        let clean_before = stored(db, clean.id).await;
        let b = db.backend;
        assert_eq!(
            c.svc.get(id).await.unwrap().options,
            legacy,
            "{b}: readable"
        );

        let sealed = c.svc.seal_plaintext_secrets().await.unwrap();
        let again = c.svc.seal_plaintext_secrets().await.unwrap();

        assert_eq!((sealed, again), (1, 0), "{b}");
        let raw = stored(db, id).await;
        assert!(raw["proxy-password"].starts_with("v1:"), "{b}: {raw:?}");
        assert_eq!(raw["k"], "v", "{b}");
        assert_eq!(stored(db, clean.id).await, clean_before, "{b}: untouched");
        assert_eq!(c.svc.get(id).await.unwrap().options, legacy, "{b}");
        assert_eq!(effective_global(&c, id).await, legacy, "{b}");
    })
    .await;
}
