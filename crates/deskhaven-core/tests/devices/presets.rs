//! `preset-*` fields: applied with the first stored sysinfo, never replacing a value.

#![allow(clippy::unwrap_used, clippy::panic)]

use deskhaven_core::addressbooks::{Actor, PeerPatch, PeerSecret, Tag};
use deskhaven_core::devices::{AddressBookPreset, PRESET_TAG_COLOR, Presets, SysinfoOutcome};
use deskhaven_core::pagination::PageRequest;
use deskhaven_core::strategies::Target;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::json;
use zeroize::Zeroizing;

use super::support::{Ctx, ID, ctx, report, sysinfo};

fn all_presets() -> Presets {
    Presets {
        user_name: Some("Alice".to_owned()),
        device_group_name: Some("Office".to_owned()),
        strategy_name: Some("Locked".to_owned()),
        note: Some("front desk".to_owned()),
        address_book: Some(AddressBookPreset {
            name: "team".to_owned(),
            tag: Some("printers".to_owned()),
            alias: Some("Reception".to_owned()),
            password: Some(Zeroizing::new("s3cret-pass".to_owned())),
            note: Some("ab note".to_owned()),
        }),
    }
}

/// Everything the presets name exists.
async fn targets(c: &Ctx) -> (i64, i64, i64, String) {
    let alice = c.user("alice").await.id;
    let group = c.devices.create_group("Office", "").await.unwrap();
    let strategy = c.strategy("Locked").await.id;
    let book = c.shared_book("Team").await.guid;
    (alice, group, strategy, book)
}

#[tokio::test]
async fn the_first_sysinfo_applies_every_preset() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let device_id = c.device(ID).await;
        let (alice, group, strategy, book) = targets(&c).await;

        // Act
        let outcome = c.report(report(ID, sysinfo("pc"), all_presets())).await;

        // Assert
        let b = db.backend;
        assert_eq!(outcome, SysinfoOutcome::Stored, "{b}");
        let row = c.row(ID).await;
        assert_eq!(row.owner_user_id, Some(alice), "{b}");
        assert_eq!(row.device_group_id, Some(group), "{b}");
        assert_eq!(row.note.as_deref(), Some("front desk"), "{b}");
        let assigned = c.strategies.assignment(Target::Device(device_id)).await;
        assert_eq!(assigned.unwrap(), Some(strategy), "{b}");
        let peers = c.books.peers(Actor::Admin, &book, PageRequest::default());
        let peers = peers.await.unwrap().items;
        assert_eq!(peers.len(), 1, "{b}");
        let peer = &peers[0];
        assert_eq!(peer.id, ID, "{b}");
        assert_eq!(
            (peer.alias.as_str(), peer.note.as_str()),
            ("Reception", "ab note"),
            "{b}"
        );
        assert_eq!(peer.tags, ["printers"], "{b}");
        assert_eq!(peer.hostname, "pc", "{b}");
        let password = PeerSecret::Password(Zeroizing::new("s3cret-pass".to_owned()));
        assert!(
            peer.secret == password,
            "{b}: shared-book password stored (encrypted)"
        );
        let tags = c.books.tags(Actor::Admin, &book).await.unwrap();
        let printers = Tag {
            name: "printers".to_owned(),
            color: PRESET_TAG_COLOR,
        };
        assert_eq!(tags, [printers], "{b}");
    })
    .await;
}

/// Presets are only read with the first stored sysinfo: a later upload (changed content)
/// carrying them changes nothing but the sysinfo.
#[tokio::test]
async fn presets_after_the_first_sysinfo_are_ignored() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let device_id = c.device(ID).await;
        let (_, _, _, book) = targets(&c).await;
        c.report(report(ID, sysinfo("pc"), Presets::default()))
            .await;

        // Act
        let outcome = c.report(report(ID, sysinfo("pc-2"), all_presets())).await;

        // Assert
        let b = db.backend;
        assert_eq!(outcome, SysinfoOutcome::Stored, "{b}");
        let row = c.row(ID).await;
        assert_eq!(row.hostname.as_deref(), Some("pc-2"), "{b}");
        let placed = (row.owner_user_id, row.device_group_id, row.note);
        assert_eq!(placed, (None, None, None), "{b}");
        let assigned = c.strategies.assignment(Target::Device(device_id)).await;
        assert_eq!(assigned.unwrap(), None, "{b}");
        let peers = c.books.peers(Actor::Admin, &book, PageRequest::default());
        assert_eq!(peers.await.unwrap().total, 0, "{b}");
    })
    .await;
}

/// What an administrator (or `--assign`) set before the first sysinfo stays.
#[tokio::test]
async fn presets_never_replace_existing_values() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let device_id = c.device(ID).await;
        let (_, _, _, book) = targets(&c).await;
        let bob = c.user("bob").await.id;
        let lab = c.devices.create_group("Lab", "").await.unwrap();
        let open = c.strategy("Open").await.id;
        c.strategies
            .assign(Target::Device(device_id), open)
            .await
            .unwrap();
        preassign(&c, bob, lab).await;
        let existing = PeerPatch::from_json(json!({ "id": ID, "alias": "Kept" })).unwrap();
        c.books
            .add_peer(Actor::Admin, &book, existing)
            .await
            .unwrap();

        // Act
        let outcome = c.report(report(ID, sysinfo("pc"), all_presets())).await;

        // Assert
        let b = db.backend;
        assert_eq!(outcome, SysinfoOutcome::Stored, "{b}");
        let row = c.row(ID).await;
        assert_eq!(row.owner_user_id, Some(bob), "{b}");
        assert_eq!(row.device_group_id, Some(lab), "{b}");
        assert_eq!(row.note.as_deref(), Some("set by admin"), "{b}");
        let assigned = c.strategies.assignment(Target::Device(device_id)).await;
        assert_eq!(assigned.unwrap(), Some(open), "{b}");
        let peers = c.books.peers(Actor::Admin, &book, PageRequest::default());
        let peers = peers.await.unwrap().items;
        assert_eq!(peers.len(), 1, "{b}");
        assert_eq!(peers[0].alias, "Kept", "{b}");
    })
    .await;
}

/// Owner, device group and note of a device, set directly (the console's job, M3-05).
async fn preassign(c: &Ctx, owner: i64, group: i64) {
    use deskhaven_db::entity::device;
    use sea_orm::{ActiveModelTrait, ActiveValue::Set, IntoActiveModel};
    let mut row = c.row(ID).await.into_active_model();
    row.owner_user_id = Set(Some(owner));
    row.device_group_id = Set(Some(group));
    row.note = Set(Some("set by admin".to_owned()));
    let _: device::Model = row.update(&c.conn).await.unwrap();
}

/// Names that match nothing are skipped; the sysinfo is stored all the same.
#[tokio::test]
async fn unknown_preset_names_are_ignored() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let device_id = c.device(ID).await;

        // Act
        let outcome = c.report(report(ID, sysinfo("pc"), all_presets())).await;

        // Assert
        let b = db.backend;
        assert_eq!(outcome, SysinfoOutcome::Stored, "{b}");
        let row = c.row(ID).await;
        assert_eq!(row.hostname.as_deref(), Some("pc"), "{b}");
        assert_eq!(
            (row.owner_user_id, row.device_group_id),
            (None, None),
            "{b}"
        );
        assert_eq!(
            row.note.as_deref(),
            Some("front desk"),
            "{b}: the note needs no target"
        );
        let assigned = c.strategies.assignment(Target::Device(device_id)).await;
        assert_eq!(assigned.unwrap(), None, "{b}");
    })
    .await;
}

/// Placing a device (owner, device group) changes what strategies apply to it, so the
/// strategy cache is invalidated after the commit.
#[tokio::test]
async fn placing_a_device_invalidates_the_strategy_cache() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.device(ID).await;
        c.user("alice").await;
        let warm = c.strategies.effective(&Default::default()).await.unwrap();
        assert!(c.strategies.cache().is_loaded());
        let presets = Presets {
            user_name: Some("alice".to_owned()),
            ..Default::default()
        };

        // Act
        c.report(report(ID, sysinfo("pc"), presets)).await;

        // Assert
        let b = db.backend;
        assert!(!c.strategies.cache().is_loaded(), "{b}");
        assert!(c.strategies.cache().version() > warm.modified_at, "{b}");
    })
    .await;
}
