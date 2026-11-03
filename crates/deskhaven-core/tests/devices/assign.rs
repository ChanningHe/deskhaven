//! `--assign` (M2-19, 03 §2.9): an API token places a device by name: owner, device group,
//! note, strategy, address book entry and the device names. Unlike presets it replaces what
//! is set, and an unknown name fails the whole request before anything changes.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::devices::{
    AddressBookPreset, DeviceAssignment, DeviceError, Heartbeat, HeartbeatOutcome, HeartbeatReply,
};
use deskhaven_core::strategies::Target;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use zeroize::Zeroizing;

use super::support::{Ctx, ID, Targets, b64, ctx, uuid_of};

fn assignment() -> DeviceAssignment {
    DeviceAssignment {
        rustdesk_id: ID.to_owned(),
        uuid: b64(&uuid_of(ID)),
        ..DeviceAssignment::default()
    }
}

fn everything() -> DeviceAssignment {
    DeviceAssignment {
        user_name: Some("Alice".to_owned()),
        strategy_name: Some("Locked".to_owned()),
        device_group_name: Some("Office".to_owned()),
        note: Some("front desk".to_owned()),
        device_username: Some("frontdesk".to_owned()),
        device_name: Some("reception-01".to_owned()),
        address_book: Some(AddressBookPreset {
            name: "team".to_owned(),
            tag: Some("printers".to_owned()),
            alias: Some("Reception".to_owned()),
            password: Some(Zeroizing::new("s3cret-pass".to_owned())),
            note: Some("ab note".to_owned()),
        }),
        ..assignment()
    }
}

impl Ctx {
    async fn assign(&self, a: DeviceAssignment) -> Result<(), DeviceError> {
        self.devices.assign(a).await
    }

    /// One heartbeat of [`ID`] holding strategy version `modified_at`, 3 s after the last.
    async fn beat(&self, modified_at: i64) -> HeartbeatReply {
        self.clock.advance(Duration::from_secs(3));
        let beat = Heartbeat {
            rustdesk_id: ID.to_owned(),
            uuid: b64(&uuid_of(ID)),
            client: ClientAddr {
                addr: IpAddr::V4(Ipv4Addr::new(198, 51, 100, 4)),
                is_proxy_fallback: false,
            },
            conns: Vec::new(),
            modified_at,
        };
        match self.devices.heartbeat(beat).await.unwrap() {
            HeartbeatOutcome::Reply(reply) => reply,
            other => panic!("expected a reply, got {other:?}"),
        }
    }
}

/// Nothing placed on device [`ID`] (`devices.id` = `device_id`).
async fn assert_untouched(c: &Ctx, t: &Targets, device_id: i64, b: impl std::fmt::Display) {
    let row = c.row(ID).await;
    let placed = (
        row.owner_user_id,
        row.device_group_id,
        row.note,
        row.hostname,
    );
    assert_eq!(placed, (None, None, None, None), "{b}");
    let assigned = c.strategies.assignment(Target::Device(device_id)).await;
    assert_eq!(assigned.unwrap(), None, "{b}");
    assert!(c.book_peers(&t.book).await.is_empty(), "{b}");
}

#[tokio::test]
async fn assign_places_the_device_by_name() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let device_id = c.device(ID).await;
        let t = c.targets().await;

        // Act
        c.assign(everything()).await.unwrap();

        // Assert
        let b = db.backend;
        let row = c.row(ID).await;
        assert_eq!(row.owner_user_id, Some(t.alice), "{b}");
        assert_eq!(row.device_group_id, Some(t.group), "{b}");
        assert_eq!(row.note.as_deref(), Some("front desk"), "{b}");
        assert_eq!(row.hostname.as_deref(), Some("reception-01"), "{b}");
        assert_eq!(row.username.as_deref(), Some("frontdesk"), "{b}");
        let assigned = c.strategies.assignment(Target::Device(device_id)).await;
        assert_eq!(assigned.unwrap(), Some(t.strategy), "{b}");
        let peers = c.book_peers(&t.book).await;
        assert_eq!(peers.len(), 1, "{b}");
        let entry = (
            peers[0].id.as_str(),
            peers[0].alias.as_str(),
            peers[0].note.as_str(),
            peers[0].hostname.as_str(),
        );
        assert_eq!(entry, (ID, "Reception", "ab note", "reception-01"), "{b}");
        assert_eq!(peers[0].tags, ["printers"], "{b}");
    })
    .await;
}

#[tokio::test]
async fn assign_replaces_an_existing_placement() {
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
        c.assign(DeviceAssignment {
            address_book: None,
            ..everything()
        })
        .await
        .unwrap();

        // Assert
        let b = db.backend;
        let row = c.row(ID).await;
        assert_eq!(row.owner_user_id, Some(t.alice), "{b}");
        assert_eq!(row.device_group_id, Some(t.group), "{b}");
        assert_eq!(row.note.as_deref(), Some("front desk"), "{b}");
        let assigned = c.strategies.assignment(Target::Device(device_id)).await;
        assert_eq!(assigned.unwrap(), Some(t.strategy), "{b}");
    })
    .await;
}

#[tokio::test]
async fn an_unknown_name_fails_and_changes_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let device_id = c.device(ID).await;
        let t = c.targets().await;
        let unknown = "nobody".to_owned();
        let book = |name: &str| AddressBookPreset {
            name: name.to_owned(),
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
                    address_book: Some(book(&unknown)),
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

#[tokio::test]
async fn an_unknown_device_or_a_wrong_uuid_is_refused() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        c.device(ID).await;
        c.targets().await;
        let cases = [
            DeviceAssignment {
                rustdesk_id: "987654321".to_owned(),
                ..everything()
            },
            DeviceAssignment {
                uuid: b64(b"another-machine"),
                ..everything()
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

#[tokio::test]
async fn an_entry_the_book_already_has_is_left_as_it_is() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        c.device(ID).await;
        let t = c.targets().await;
        c.assign(everything()).await.unwrap();
        let again = DeviceAssignment {
            address_book: Some(AddressBookPreset {
                name: "Team".to_owned(),
                alias: Some("Changed".to_owned()),
                ..AddressBookPreset::default()
            }),
            ..assignment()
        };

        c.assign(again).await.unwrap();

        let peers = c.book_peers(&t.book).await;
        assert_eq!(peers.len(), 1, "{}", db.backend);
        assert_eq!(peers[0].alias, "Reception", "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn the_next_heartbeat_delivers_the_assigned_strategy() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.device(ID).await;
        c.targets().await;
        let before = c.beat(0).await;
        let held = before.strategy.map_or(0, |push| push.modified_at);

        // Act
        c.assign(DeviceAssignment {
            strategy_name: Some("Locked".to_owned()),
            ..assignment()
        })
        .await
        .unwrap();
        let after = c.beat(held).await;

        // Assert
        let Some(push) = after.strategy else {
            panic!("{}: no strategy push after the assignment", db.backend);
        };
        assert_ne!(push.modified_at, held, "{}", db.backend);
        assert_eq!(
            push.config_options
                .get("enable-file-transfer")
                .map(String::as_str),
            Some("N"),
            "{}",
            db.backend
        );
    })
    .await;
}
