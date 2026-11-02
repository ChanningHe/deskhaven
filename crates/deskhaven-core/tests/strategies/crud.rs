#![allow(clippy::unwrap_used, clippy::panic)]

use deskhaven_core::strategies::{
    KEY_MAX_LEN, MAX_OPTIONS, NewStrategy, Options, StrategyError, StrategyUpdate, VALUE_MAX_LEN,
};
use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::entity::strategy;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;
use sea_orm::{ActiveModelTrait, ActiveValue::Set};

use super::support::{ctx, opts};

#[tokio::test]
async fn strategies_can_be_created_read_updated_listed_and_deleted() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;

        let new = NewStrategy {
            note: "locked down".to_owned(),
            ..NewStrategy::new(" kiosk ", opts(&[("enable-file-transfer", "N"), ("x", "")]))
        };
        let kiosk = c.svc.create(new).await.unwrap();
        let office = c.strategy("office", &[]).await;

        assert_eq!(kiosk.name, "kiosk");
        assert_eq!(kiosk.managed_by, ManagedBy::Ui);
        assert_eq!(
            kiosk.options,
            opts(&[("enable-file-transfer", "N"), ("x", "")])
        );
        assert_eq!(c.svc.get(kiosk.id).await.unwrap(), kiosk);
        assert_eq!(c.svc.get_by_name("kiosk").await.unwrap(), kiosk);
        let names: Vec<_> = c
            .svc
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, ["kiosk", "office"]);

        c.clock.advance(std::time::Duration::from_secs(5));
        let update = StrategyUpdate {
            name: Some("kiosk-v2".to_owned()),
            options: Some(opts(&[("enable-audio", "Y")])),
            note: None,
        };
        let updated = c.svc.update(kiosk.id, update).await.unwrap();
        assert_eq!(updated.name, "kiosk-v2");
        assert_eq!(updated.options, opts(&[("enable-audio", "Y")]));
        assert_eq!(updated.note, "locked down");
        assert!(updated.updated_at.timestamp_millis() > kiosk.updated_at.timestamp_millis());
        assert_eq!(updated.created_at, kiosk.created_at);

        c.svc.delete(office.id).await.unwrap();
        assert!(matches!(
            c.svc.get(office.id).await,
            Err(StrategyError::NotFound)
        ));
        assert!(matches!(
            c.svc.delete(office.id).await,
            Err(StrategyError::NotFound)
        ));
        let update = StrategyUpdate::default();
        assert!(matches!(
            c.svc.update(office.id, update).await,
            Err(StrategyError::NotFound)
        ));
        assert!(matches!(
            c.svc.get_by_name("office").await,
            Err(StrategyError::NotFound)
        ));
    })
    .await;
}

#[tokio::test]
async fn strategy_names_are_unique_and_validated() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let a = c.strategy("a", &[]).await;
        c.strategy("b", &[]).await;

        let duplicate = c.svc.create(NewStrategy::new("a", Options::new())).await;
        assert!(
            matches!(duplicate, Err(StrategyError::Conflict { field: "name" })),
            "{duplicate:?}"
        );
        let rename = StrategyUpdate {
            name: Some("b".to_owned()),
            ..StrategyUpdate::default()
        };
        let clash = c.svc.update(a.id, rename).await;
        assert!(
            matches!(clash, Err(StrategyError::Conflict { field: "name" })),
            "{clash:?}"
        );
        // The failed rename left the strategy as it was.
        assert_eq!(c.svc.get(a.id).await.unwrap().name, "a");

        for bad in ["", "   ", "tab\there", &"s".repeat(65)] {
            let result = c.svc.create(NewStrategy::new(bad, Options::new())).await;
            assert!(
                matches!(result, Err(StrategyError::Invalid { field: "name", .. })),
                "{bad:?}: {result:?}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn options_are_validated_on_create_and_update_and_empty_values_are_kept() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let too_many: Options = (0..=MAX_OPTIONS)
            .map(|i| (format!("k{i}"), String::new()))
            .collect();
        let bad = [
            opts(&[("", "v")]),
            opts(&[("two words", "v")]),
            opts(&[(&"k".repeat(KEY_MAX_LEN + 1), "v")]),
            opts(&[("k", &"v".repeat(VALUE_MAX_LEN + 1))]),
            opts(&[("k", "multi\nline")]),
            too_many,
        ];
        let good = c.strategy("good", &[("k", "")]).await;

        for options in bad {
            let create = c.svc.create(NewStrategy::new("bad", options.clone())).await;
            assert!(
                matches!(
                    create,
                    Err(StrategyError::Invalid {
                        field: "options",
                        ..
                    })
                ),
                "{create:?}"
            );
            let update = StrategyUpdate {
                options: Some(options),
                ..StrategyUpdate::default()
            };
            let update = c.svc.update(good.id, update).await;
            assert!(
                matches!(
                    update,
                    Err(StrategyError::Invalid {
                        field: "options",
                        ..
                    })
                ),
                "{update:?}"
            );
        }
        assert_eq!(
            c.svc.get(good.id).await.unwrap().options,
            opts(&[("k", "")])
        );
        assert!(c.svc.get_by_name("bad").await.is_err());
    })
    .await;
}

#[tokio::test]
async fn config_managed_strategies_are_read_only() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let now = time::now();
        let row = strategy::ActiveModel {
            name: Set("from-file".to_owned()),
            options: Set("{}".to_owned()),
            note: Set(String::new()),
            managed_by: Set(ManagedBy::Config),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        let id = row.insert(&db.conn).await.unwrap().id;

        let rename = StrategyUpdate {
            name: Some("x".to_owned()),
            ..StrategyUpdate::default()
        };
        assert!(matches!(
            c.svc.update(id, rename).await,
            Err(StrategyError::ManagedByConfig)
        ));
        assert!(matches!(
            c.svc.delete(id).await,
            Err(StrategyError::ManagedByConfig)
        ));
        // Assigning it is not changing it.
        c.svc
            .assign(deskhaven_core::strategies::Target::Global, id)
            .await
            .unwrap();
    })
    .await;
}

#[tokio::test]
async fn unreadable_stored_options_are_reported_not_hidden() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let now = time::now();
        let row = strategy::ActiveModel {
            name: Set("broken".to_owned()),
            options: Set("[1]".to_owned()),
            note: Set(String::new()),
            managed_by: Set(ManagedBy::Ui),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        let id = row.insert(&db.conn).await.unwrap().id;

        assert!(matches!(c.svc.get(id).await, Err(StrategyError::Corrupt { strategy_id }) if strategy_id == id));
        c.svc.assign(deskhaven_core::strategies::Target::Global, id).await.unwrap();
        let inputs = super::support::device_inputs(1, None, None);
        let effective = c.svc.effective(&inputs).await;
        assert!(matches!(effective, Err(StrategyError::Corrupt { .. })), "{effective:?}");
    })
    .await;
}
