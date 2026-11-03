//! `--assign` failures (M2-19 review): the device is resolved first, every step runs in one
//! transaction with its audit row, so any failure changes nothing and leaves no audit row.

#![allow(clippy::unwrap_used, clippy::panic)]

use deskhaven_core::addressbooks::{Actor, PeerPatch};
use deskhaven_core::devices::{AddressBookPreset, DeviceAssignment, DeviceError};
use deskhaven_core::strategies::Target;
use deskhaven_db::entity::console_audit::ActorKind;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::{Value, json};

use super::assign::{assignment, everything};
use super::deploy::TOKEN;
use super::support::{Ctx, ID, Targets, b64, ctx};

/// Nothing placed on device [`ID`] (`devices.id` = `device_id`), and nothing audited.
async fn assert_untouched(c: &Ctx, t: &Targets, device_id: i64, b: impl std::fmt::Display) {
    let row = c.row(ID).await;
    let placed = (
        row.owner_user_id,
        row.device_group_id,
        row.note,
        row.display_name,
    );
    assert_eq!(placed, (None, None, None, None), "{b}");
    let assigned = c.strategies.assignment(Target::Device(device_id)).await;
    assert_eq!(assigned.unwrap(), None, "{b}");
    assert!(c.book_peers(&t.book).await.is_empty(), "{b}");
    assert!(c.audit_rows().await.is_empty(), "{b}");
}

#[tokio::test]
async fn an_unknown_name_fails_and_changes_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let device_id = c.device(ID).await;
        let t = c.targets().await;
        let unknown = "nobody".to_owned();
        let book = AddressBookPreset {
            name: unknown.clone(),
            ..AddressBookPreset::default()
        };
        let cases = [
            (
                "user_name",
                DeviceAssignment {
                    user_name: Some(unknown.clone()),
                    ..everything()
                },
            ),
            (
                "device_group_name",
                DeviceAssignment {
                    device_group_name: Some(unknown.clone()),
                    ..everything()
                },
            ),
            (
                "strategy_name",
                DeviceAssignment {
                    strategy_name: Some(unknown.clone()),
                    ..everything()
                },
            ),
            (
                "address_book_name",
                DeviceAssignment {
                    address_book: Some(book),
                    ..everything()
                },
            ),
        ];

        for (field, case) in cases {
            let err = c.assign(case).await.unwrap_err();
            assert!(
                matches!(err, DeviceError::NameNotFound { field: f, .. } if f == field),
                "{}: {field}: {err:?}",
                db.backend
            );
            assert_untouched(&c, &t, device_id, db.backend).await;
        }
    })
    .await;
}

/// The device is resolved before any name: an unknown device with unknown names is a 404,
/// so a token cannot probe which users, groups, strategies or books exist.
#[tokio::test]
async fn an_unknown_device_is_refused_before_any_name_is_looked_up() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        c.device(ID).await;
        c.targets().await;
        let nobody = DeviceAssignment {
            user_name: Some("nobody".to_owned()),
            ..everything()
        };
        let cases = [
            DeviceAssignment {
                rustdesk_id: "987654321".to_owned(),
                ..nobody.clone()
            },
            DeviceAssignment {
                uuid: b64(b"another-machine"),
                ..nobody
            },
        ];

        for case in cases {
            let err = c.assign(case).await.unwrap_err();
            assert!(
                matches!(err, DeviceError::UnknownDevice),
                "{}: {err:?}",
                db.backend
            );
        }
        assert_eq!(c.row(ID).await.owner_user_id, None, "{}", db.backend);
    })
    .await;
}

/// API tokens travel in scripts: the uuid is checked even with `devices.verify_uuid` off.
#[tokio::test]
async fn the_uuid_is_checked_even_with_uuid_verification_off() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        c.device(ID).await;
        c.targets().await;
        c.set("devices.verify_uuid", json!(false)).await;

        let wrong = DeviceAssignment {
            uuid: b64(b"another-machine"),
            ..everything()
        };
        let err = c.assign(wrong).await.unwrap_err();

        assert!(
            matches!(err, DeviceError::UnknownDevice),
            "{}: {err:?}",
            db.backend
        );
        assert_eq!(c.row(ID).await.owner_user_id, None, "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn empty_and_malformed_assignments_are_invalid() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        c.device(ID).await;
        c.targets().await;
        let cases = [
            assignment(),
            DeviceAssignment {
                note: Some("bad\u{0}note".to_owned()),
                ..everything()
            },
            DeviceAssignment {
                device_name: Some("x".repeat(1000)),
                ..everything()
            },
        ];

        for case in cases {
            let err = c.assign(case).await.unwrap_err();
            assert!(
                matches!(err, DeviceError::Invalid { .. }),
                "{}: {err:?}",
                db.backend
            );
        }
        assert_eq!(c.row(ID).await.owner_user_id, None, "{}", db.backend);
    })
    .await;
}

/// A full book is found out after the row was locked: the whole assignment rolls back.
#[tokio::test]
async fn a_full_book_fails_the_assignment_and_changes_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let device_id = c.device(ID).await;
        let t = c.targets().await;
        c.set("addressbook.max_peers", json!(1)).await;
        let filler = PeerPatch::from_json(json!({ "id": "999999999" })).unwrap();
        c.books
            .add_peer(Actor::Admin, &t.book, filler)
            .await
            .unwrap();

        let err = c.assign(everything()).await.unwrap_err();

        let b = db.backend;
        assert!(
            matches!(
                err,
                DeviceError::Invalid {
                    field: "address_book",
                    ..
                }
            ),
            "{b}: {err:?}"
        );
        let row = c.row(ID).await;
        assert_eq!((row.owner_user_id, row.note), (None, None), "{b}");
        let assigned = c.strategies.assignment(Target::Device(device_id)).await;
        assert_eq!(assigned.unwrap(), None, "{b}");
        assert_eq!(c.book_peers(&t.book).await.len(), 1, "{b}");
        assert!(c.audit_rows().await.is_empty(), "{b}");
    })
    .await;
}

/// One audit row per assignment, as the token, with the placement before and after.
#[tokio::test]
async fn an_assignment_is_audited_with_the_previous_placement() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let device_id = c.device(ID).await;
        let t = c.targets().await;
        let bob = c.user("bob").await.id;
        let lab = c.devices.create_group("Lab", "").await.unwrap();
        c.preassign(bob, lab).await;
        let other = c.strategy("Other").await.id;
        c.strategies
            .assign(Target::Device(device_id), other)
            .await
            .unwrap();

        // Act
        c.assign(everything()).await.unwrap();

        // Assert
        let b = db.backend;
        let audit = c.audit_rows().await;
        assert_eq!(audit.len(), 1, "{b}");
        let entry = &audit[0];
        assert_eq!(
            (entry.actor_kind, entry.actor_id),
            (ActorKind::ApiToken, Some(TOKEN))
        );
        assert_eq!(
            (entry.action.as_str(), entry.target_id.as_str()),
            ("device.assign", ID)
        );
        let parse =
            |s: &Option<String>| -> Value { serde_json::from_str(s.as_deref().unwrap()).unwrap() };
        let (before, after) = (parse(&entry.before), parse(&entry.after));
        assert_eq!(before["owner_user_id"], json!(bob), "{b}");
        assert_eq!(before["device_group_id"], json!(lab), "{b}");
        assert_eq!(before["note"], json!("set by admin"), "{b}");
        assert_eq!(before["strategy_id"], json!(other), "{b}");
        assert_eq!(after["owner_user_id"], json!(t.alice), "{b}");
        assert_eq!(after["strategy_id"], json!(t.strategy), "{b}");
        assert!(!entry.after.clone().unwrap().contains("s3cret-pass"), "{b}");
    })
    .await;
}
