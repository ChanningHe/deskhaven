#![allow(clippy::unwrap_used, clippy::panic)]

use deskhaven_core::strategies::{
    KEY_MAX_LEN, MAX_OPTIONS, NewStrategy, Options, StrategyError, StrategyUpdate, Target,
    VALUE_MAX_LEN,
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
            ..StrategyUpdate::default()
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

/// A strategy whose stored options were damaged outside the service.
async fn insert_corrupt(db: &TestDb, name: &str) -> i64 {
    insert_stored(db, name, "[1]").await
}

/// A strategy row with `stored` options, written past the service.
async fn insert_stored(db: &TestDb, name: &str, stored: &str) -> i64 {
    let now = time::now();
    let row = strategy::ActiveModel {
        name: Set(name.to_owned()),
        options: Set(stored.to_owned()),
        note: Set(String::new()),
        managed_by: Set(ManagedBy::Ui),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    };
    row.insert(&db.conn).await.unwrap().id
}

#[tokio::test]
async fn a_corrupt_strategy_is_listed_flagged_and_does_not_break_the_others() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let fine = c.strategy("fine", &[("k", "v")]).await;
        let broken = insert_corrupt(db, "broken").await;

        // Admins can still see it (flagged, options empty), so they can fix or delete it.
        let all = c.svc.list().await.unwrap();
        let flags: Vec<_> = all
            .iter()
            .map(|s| (s.name.as_str(), s.options_corrupt))
            .collect();
        assert_eq!(flags, [("broken", true), ("fine", false)]);
        let got = c.svc.get(broken).await.unwrap();
        assert!(got.options_corrupt && got.options.is_empty());
        assert!(!c.svc.get(fine.id).await.unwrap().options_corrupt);
    })
    .await;
}

#[tokio::test]
async fn effective_skips_a_corrupt_assigned_strategy_and_caches_the_result() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let device = c.device("1").await;
        let fine = c.strategy("fine", &[("k", "v")]).await;
        let broken = insert_corrupt(db, "broken").await;
        c.svc.assign(Target::Global, broken).await.unwrap();
        c.assign(Target::Device(device), &fine).await;
        let inputs = super::support::device_inputs(device, None, None);

        let first = c.svc.effective(&inputs).await.unwrap();
        let second = c.svc.effective(&inputs).await.unwrap();

        assert_eq!(first.options, opts(&[("k", "v")]));
        let applied: Vec<_> = first.applied.iter().map(|a| a.strategy_id).collect();
        assert_eq!(applied, [fine.id]);
        assert_eq!(second, first);
        assert_eq!(
            c.svc.cache().loads(),
            1,
            "a bad row must not force a reload per call"
        );
    })
    .await;
}

#[tokio::test]
async fn a_corrupt_strategy_can_be_repaired_or_deleted() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let (repaired, doomed) = (insert_corrupt(db, "a").await, insert_corrupt(db, "b").await);
        c.svc.assign(Target::Global, repaired).await.unwrap();
        let inputs = super::support::device_inputs(1, None, None);
        assert!(c.options(&inputs).await.is_empty());

        let rename = StrategyUpdate {
            name: Some("a2".to_owned()),
            ..StrategyUpdate::default()
        };
        assert!(
            c.svc
                .update(repaired, rename)
                .await
                .unwrap()
                .options_corrupt
        );
        let fix = StrategyUpdate {
            options: Some(opts(&[("k", "v")])),
            ..StrategyUpdate::default()
        };
        assert!(!c.svc.update(repaired, fix).await.unwrap().options_corrupt);
        assert_eq!(c.options(&inputs).await, opts(&[("k", "v")]));

        c.svc.delete(doomed).await.unwrap();
        assert_eq!(c.svc.list().await.unwrap().len(), 1);
    })
    .await;
}

const SPACED_WHITELIST: &str = " 10.0.0.0/8, 192.0.2.1 ,,";
const CLEAN_WHITELIST: &str = "10.0.0.0/8,192.0.2.1";

/// Clients before rustdesk 78450f4 (#16692) do not trim IP whitelist entries, so every write
/// (create, replacement, patch) stores the whitelist without spaces or empty entries, and
/// devices get it that way. Other options keep their spaces.
#[tokio::test]
async fn ip_whitelist_entries_are_stored_and_delivered_without_spaces() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let b = db.backend;
        let inputs = super::support::device_inputs(1, None, None);
        let spaced = opts(&[("whitelist", SPACED_WHITELIST), ("k", " v ")]);
        let clean = opts(&[("whitelist", CLEAN_WHITELIST), ("k", " v ")]);

        let created = c.svc.create(NewStrategy::new("s", spaced.clone())).await;
        let created = created.unwrap();
        c.svc.assign(Target::Global, created.id).await.unwrap();

        assert_eq!(created.options, clean, "{b}: create");
        assert_eq!(c.svc.get(created.id).await.unwrap().options, clean, "{b}");
        assert_eq!(c.options(&inputs).await, clean, "{b}: delivered");

        let replace = StrategyUpdate {
            options: Some(spaced),
            ..StrategyUpdate::default()
        };
        let replaced = c.svc.update(created.id, replace).await.unwrap();
        assert_eq!(replaced.options, clean, "{b}: replacement");
        let patch = StrategyUpdate {
            options_patch: [("whitelist".to_owned(), Some(" , ".to_owned()))].into(),
            ..StrategyUpdate::default()
        };
        let patched = c.svc.update(created.id, patch).await.unwrap();
        assert_eq!(patched.options["whitelist"], "", "{b}: patch");
        assert_eq!(c.options(&inputs).await["whitelist"], "", "{b}: delivered");
    })
    .await;
}

/// Rows stored before writes were normalized are delivered normalized; the console shows
/// them as stored until the next write.
#[tokio::test]
async fn ip_whitelist_of_an_older_row_is_delivered_without_spaces() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let b = db.backend;
        let stored = format!(r#"{{"whitelist":"{SPACED_WHITELIST}"}}"#);
        let id = insert_stored(db, "old", &stored).await;
        c.svc.assign(Target::Global, id).await.unwrap();
        let inputs = super::support::device_inputs(1, None, None);

        let delivered = c.options(&inputs).await;

        assert_eq!(delivered, opts(&[("whitelist", CLEAN_WHITELIST)]), "{b}");
        let shown = c.svc.get(id).await.unwrap().options;
        assert_eq!(shown, opts(&[("whitelist", SPACED_WHITELIST)]), "{b}");
    })
    .await;
}
