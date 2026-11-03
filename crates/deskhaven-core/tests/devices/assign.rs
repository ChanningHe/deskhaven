//! `--assign` (M2-19, 03 §2.9): an API token places a device by name: owner, device group,
//! note, strategy, address book entry and the administrator's names for the device. Unlike
//! presets it replaces what is set. Failures: `assign_failures`.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use deskhaven_core::addressbooks::{Actor, PeerPatch};
use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::devices::{
    AddressBookPreset, DeviceAssignment, DeviceError, Heartbeat, HeartbeatOutcome, HeartbeatReply,
    Presets, SysinfoOutcome,
};
use deskhaven_core::strategies::Target;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use zeroize::Zeroizing;

use super::deploy::{IP, requester};
use super::support::{Ctx, ID, b64, ctx, report, sysinfo, uuid_of};

pub fn assignment() -> DeviceAssignment {
    DeviceAssignment {
        rustdesk_id: ID.to_owned(),
        uuid: b64(&uuid_of(ID)),
        ..DeviceAssignment::default()
    }
}

pub fn everything() -> DeviceAssignment {
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
    pub async fn assign(&self, a: DeviceAssignment) -> Result<(), DeviceError> {
        self.devices.assign(a, requester(IP)).await
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
        assert_eq!(row.display_name.as_deref(), Some("reception-01"), "{b}");
        assert_eq!(row.display_username.as_deref(), Some("frontdesk"), "{b}");
        assert_eq!(
            (row.hostname, row.username),
            (None, None),
            "{b}: self-reported only"
        );
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

/// The names an administrator gave survive the device's own sysinfo uploads, which right
/// after a deploy carry the machine's hostname and user.
#[tokio::test]
async fn the_assigned_names_survive_the_next_sysinfo() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        c.device(ID).await;
        c.targets().await;
        c.assign(everything()).await.unwrap();

        let outcome = c
            .report(report(ID, sysinfo("DESKTOP-4F2K"), Presets::default()))
            .await;

        let b = db.backend;
        assert_eq!(outcome, SysinfoOutcome::Stored, "{b}");
        let row = c.row(ID).await;
        assert_eq!(row.hostname.as_deref(), Some("DESKTOP-4F2K"), "{b}");
        assert_eq!(row.display_name.as_deref(), Some("reception-01"), "{b}");
        assert_eq!(row.display_username.as_deref(), Some("frontdesk"), "{b}");
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

/// The address book entry follows the same replace semantics: an existing entry takes the
/// given alias and password, and the tag is added to its tags.
#[tokio::test]
async fn an_existing_book_entry_is_updated() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        c.device(ID).await;
        let t = c.targets().await;
        c.assign(everything()).await.unwrap();
        let again = DeviceAssignment {
            address_book: Some(AddressBookPreset {
                name: "Team".to_owned(),
                alias: Some("Changed".to_owned()),
                tag: Some("lobby".to_owned()),
                ..AddressBookPreset::default()
            }),
            ..assignment()
        };

        c.assign(again).await.unwrap();

        let b = db.backend;
        let peers = c.book_peers(&t.book).await;
        assert_eq!(peers.len(), 1, "{b}");
        assert_eq!(peers[0].alias, "Changed", "{b}");
        assert_eq!(peers[0].note, "ab note", "{b}: not given, kept");
        assert_eq!(peers[0].tags, ["printers", "lobby"], "{b}");
        let audit = c.audit_rows().await;
        let before = audit[1].before.clone().unwrap();
        let before: serde_json::Value = serde_json::from_str(&before).unwrap();
        assert_eq!(before["address_book"]["alias"], "Reception", "{b}");
        assert!(
            !before["address_book"]["had_password"].is_null(),
            "{b}: it had one"
        );
        assert!(
            !audit[1].before.clone().unwrap().contains("s3cret-pass"),
            "{b}"
        );
    })
    .await;
}

/// The entry's hostname and username change only when the assignment names the device:
/// otherwise what an administrator wrote into the entry stays.
#[tokio::test]
async fn an_entry_keeps_its_names_unless_the_assignment_gives_some() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        c.device(ID).await;
        let t = c.targets().await;
        c.report(report(ID, sysinfo("DESKTOP-4F2K"), Presets::default()))
            .await;
        let manual = serde_json::json!({ "id": ID, "hostname": "front-pc", "username": "kim" });
        let manual = PeerPatch::from_json(manual).unwrap();
        c.books
            .add_peer(Actor::Admin, &t.book, manual)
            .await
            .unwrap();
        let alias_only = DeviceAssignment {
            address_book: Some(AddressBookPreset {
                name: "Team".to_owned(),
                alias: Some("Reception".to_owned()),
                ..AddressBookPreset::default()
            }),
            ..assignment()
        };

        c.assign(alias_only).await.unwrap();

        let b = db.backend;
        let peers = c.book_peers(&t.book).await;
        let names = (peers[0].hostname.as_str(), peers[0].username.as_str());
        assert_eq!(names, ("front-pc", "kim"), "{b}");
        assert_eq!(peers[0].alias, "Reception", "{b}");
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
