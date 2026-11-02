//! `preset-*` fields: off by default (`devices.accept_presets`), never with
//! `devices.verify_uuid` off, applied with the first stored sysinfo only, never replacing a
//! value.

#![allow(clippy::unwrap_used, clippy::panic)]

use deskhaven_core::addressbooks::{Actor, PeerPatch, Rule};
use deskhaven_core::devices::{AddressBookPreset, Presets, SysinfoOutcome};
use deskhaven_core::strategies::Target;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::json;
use zeroize::Zeroizing;

use super::support::{Ctx, ID, Targets, ctx, report, sysinfo};

pub fn all_presets() -> Presets {
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

/// Nothing is placed: owner, group, note, strategy and book entry all unset.
async fn assert_unplaced(c: &Ctx, t: &Targets, device_id: i64, b: impl std::fmt::Display) {
    let row = c.row(ID).await;
    let placed = (row.owner_user_id, row.device_group_id, row.note);
    assert_eq!(placed, (None, None, None), "{b}");
    let assigned = c.strategies.assignment(Target::Device(device_id)).await;
    assert_eq!(assigned.unwrap(), None, "{b}");
    assert!(c.book_peers(&t.book).await.is_empty(), "{b}");
}

#[tokio::test]
async fn presets_are_ignored_by_default() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let device_id = c.device(ID).await;
        let t = c.targets().await;

        // Act
        let outcome = c.report(report(ID, sysinfo("pc"), all_presets())).await;

        // Assert
        assert_eq!(outcome, SysinfoOutcome::Stored, "{}", db.backend);
        assert_eq!(c.row(ID).await.hostname.as_deref(), Some("pc"));
        assert_unplaced(&c, &t, device_id, db.backend).await;
    })
    .await;
}

/// `row`: owner (case-insensitive name), device group, note and strategy; no book entry.
#[tokio::test]
async fn row_applies_the_placement_but_not_the_address_book_entry() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let device_id = c.device(ID).await;
        let t = c.targets().await;
        c.grant(&t.book, t.alice, Rule::ReadWrite).await;
        c.accept_presets("row").await;

        // Act
        let outcome = c.report(report(ID, sysinfo("pc"), all_presets())).await;

        // Assert
        let b = db.backend;
        assert_eq!(outcome, SysinfoOutcome::Stored, "{b}");
        let row = c.row(ID).await;
        assert_eq!(row.owner_user_id, Some(t.alice), "{b}");
        assert_eq!(row.device_group_id, Some(t.group), "{b}");
        assert_eq!(row.note.as_deref(), Some("front desk"), "{b}");
        let assigned = c.strategies.assignment(Target::Device(device_id)).await;
        assert_eq!(assigned.unwrap(), Some(t.strategy), "{b}");
        assert!(c.book_peers(&t.book).await.is_empty(), "{b}");
    })
    .await;
}

/// `all`: the book entry too, written as the (preset) owner, who holds rule 2 on the book.
#[tokio::test]
async fn all_adds_the_book_entry_as_an_owner_with_write_access() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.device(ID).await;
        let t = c.targets().await;
        c.grant(&t.book, t.alice, Rule::ReadWrite).await;
        c.accept_presets("all").await;

        // Act
        c.report(report(ID, sysinfo("pc"), all_presets())).await;

        // Assert
        let b = db.backend;
        let peers = c.book_peers(&t.book).await;
        assert_eq!(peers.len(), 1, "{b}");
        let peer = &peers[0];
        assert_eq!(peer.id, ID, "{b}");
        let entry = (
            peer.alias.as_str(),
            peer.note.as_str(),
            peer.hostname.as_str(),
        );
        assert_eq!(entry, ("Reception", "ab note", "pc"), "{b}");
        assert_eq!(peer.tags, ["printers"], "{b}");
        let tags = c.books.tags(Actor::Admin, &t.book).await.unwrap();
        assert_eq!(tags.len(), 1, "{b}");
        assert_eq!(
            tags[0].color,
            deskhaven_core::devices::PRESET_TAG_COLOR,
            "{b}"
        );
    })
    .await;
}

#[tokio::test]
async fn the_book_entry_needs_an_owner_with_write_access() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.device(ID).await;
        let t = c.targets().await;
        c.grant(&t.book, t.alice, Rule::ReadOnly).await;
        c.accept_presets("all").await;
        let ownerless = Presets {
            user_name: None,
            ..all_presets()
        };

        // Act: no owner, then (another device) an owner with rule 1.
        c.report(report(ID, sysinfo("pc"), ownerless)).await;
        c.device("987654321").await;
        let read_only = report("987654321", sysinfo("pc-2"), all_presets());
        c.report(read_only).await;

        // Assert
        let b = db.backend;
        assert!(c.book_peers(&t.book).await.is_empty(), "{b}");
        let tags = c.books.tags(Actor::Admin, &t.book).await.unwrap();
        assert!(tags.is_empty(), "{b}: no tag without an entry");
        assert_eq!(c.row("987654321").await.owner_user_id, Some(t.alice), "{b}");
    })
    .await;
}

/// With uuid verification off, anyone knowing an id could send its first sysinfo.
#[tokio::test]
async fn presets_are_ignored_while_uuid_verification_is_off() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let device_id = c.device(ID).await;
        let t = c.targets().await;
        c.grant(&t.book, t.alice, Rule::ReadWrite).await;
        c.accept_presets("all").await;
        c.set("devices.verify_uuid", json!(false)).await;

        // Act
        let outcome = c.report(report(ID, sysinfo("pc"), all_presets())).await;

        // Assert
        assert_eq!(outcome, SysinfoOutcome::Stored, "{}", db.backend);
        assert_unplaced(&c, &t, device_id, db.backend).await;
    })
    .await;
}

/// Presets are only read with the first stored sysinfo.
#[tokio::test]
async fn presets_after_the_first_sysinfo_are_ignored() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let device_id = c.device(ID).await;
        let t = c.targets().await;
        c.grant(&t.book, t.alice, Rule::ReadWrite).await;
        c.accept_presets("all").await;
        c.report(report(ID, sysinfo("pc"), Presets::default()))
            .await;

        // Act
        let outcome = c.report(report(ID, sysinfo("pc-2"), all_presets())).await;

        // Assert
        assert_eq!(outcome, SysinfoOutcome::Stored, "{}", db.backend);
        assert_eq!(c.row(ID).await.hostname.as_deref(), Some("pc-2"));
        assert_unplaced(&c, &t, device_id, db.backend).await;
    })
    .await;
}

/// What an administrator set before the first sysinfo stays; an existing book entry is
/// kept as it is and no tag is created for it.
#[tokio::test]
async fn presets_never_replace_existing_values() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let device_id = c.device(ID).await;
        let t = c.targets().await;
        let bob = c.user("bob").await.id;
        c.grant(&t.book, bob, Rule::ReadWrite).await;
        let lab = c.devices.create_group("Lab", "").await.unwrap();
        let open = c.strategy("Open").await.id;
        c.strategies
            .assign(Target::Device(device_id), open)
            .await
            .unwrap();
        c.preassign(bob, lab).await;
        let existing = PeerPatch::from_json(json!({ "id": ID, "alias": "Kept" })).unwrap();
        c.books
            .add_peer(Actor::Admin, &t.book, existing)
            .await
            .unwrap();
        c.accept_presets("all").await;

        // Act
        c.report(report(ID, sysinfo("pc"), all_presets())).await;

        // Assert
        let b = db.backend;
        let row = c.row(ID).await;
        assert_eq!(row.owner_user_id, Some(bob), "{b}");
        assert_eq!(row.device_group_id, Some(lab), "{b}");
        assert_eq!(row.note.as_deref(), Some("set by admin"), "{b}");
        let assigned = c.strategies.assignment(Target::Device(device_id)).await;
        assert_eq!(assigned.unwrap(), Some(open), "{b}");
        let peers = c.book_peers(&t.book).await;
        assert_eq!((peers.len(), peers[0].alias.as_str()), (1, "Kept"), "{b}");
        assert!(peers[0].tags.is_empty(), "{b}");
        let tags = c.books.tags(Actor::Admin, &t.book).await.unwrap();
        assert!(
            tags.is_empty(),
            "{b}: no tag for an entry that was not added"
        );
    })
    .await;
}

/// A full book gets no entry; a book at its tag limit gets the entry without a new tag.
#[tokio::test]
async fn full_books_and_tag_lists_are_respected() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let t = c.targets().await;
        c.grant(&t.book, t.alice, Rule::ReadWrite).await;
        c.accept_presets("all").await;
        let tagged = c.shared_book("Tagged").await.guid;
        c.grant(&tagged, t.alice, Rule::ReadWrite).await;
        c.fill_tags(&tagged).await;
        c.set("addressbook.max_peers", json!(1)).await;
        let other = PeerPatch::from_json(json!({ "id": "111111111" })).unwrap();
        c.books
            .add_peer(Actor::Admin, &t.book, other)
            .await
            .unwrap();
        c.device(ID).await;
        c.device("987654321").await;
        let to_tagged = Presets {
            address_book: Some(AddressBookPreset {
                name: "Tagged".to_owned(),
                tag: Some("brand-new".to_owned()),
                ..Default::default()
            }),
            ..all_presets()
        };

        // Act
        c.report(report(ID, sysinfo("pc"), all_presets())).await;
        c.report(report("987654321", sysinfo("pc-2"), to_tagged))
            .await;

        // Assert
        let b = db.backend;
        let full = c.book_peers(&t.book).await;
        assert_eq!(full.len(), 1, "{b}: only the earlier peer");
        let peers = c.book_peers(&tagged).await;
        assert_eq!(peers.len(), 1, "{b}");
        assert!(peers[0].tags.is_empty(), "{b}: entry kept, tag not created");
        let tags = c.books.tags(Actor::Admin, &tagged).await.unwrap();
        assert!(tags.iter().all(|t| t.name != "brand-new"), "{b}");
    })
    .await;
}

/// Concurrent first uploads: exactly one of them is first, and its presets alone apply.
#[tokio::test]
async fn concurrent_first_sysinfos_apply_one_set_of_presets() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let device_id = c.device(ID).await;
        let t = c.targets().await;
        let bob = c.user("bob").await.id;
        let open = c.strategy("Open").await.id;
        c.accept_presets("row").await;
        let as_bob = Presets {
            user_name: Some("bob".to_owned()),
            strategy_name: Some("Open".to_owned()),
            ..Default::default()
        };
        let as_alice = Presets {
            user_name: Some("alice".to_owned()),
            strategy_name: Some("Locked".to_owned()),
            ..Default::default()
        };

        // Act
        let (a, b) = tokio::join!(
            c.devices
                .report_sysinfo(report(ID, sysinfo("from-alice"), as_alice)),
            c.devices
                .report_sysinfo(report(ID, sysinfo("from-bob"), as_bob)),
        );

        // Assert
        let be = db.backend;
        assert_eq!(
            (a.unwrap(), b.unwrap()),
            (SysinfoOutcome::Stored, SysinfoOutcome::Stored)
        );
        let row = c.row(ID).await;
        let assigned = c.strategies.assignment(Target::Device(device_id)).await;
        let assigned = assigned.unwrap();
        // The first stored upload placed the device; the second overwrote only the sysinfo.
        let expected = match row.hostname.as_deref() {
            Some("from-bob") => (Some(t.alice), Some(t.strategy)),
            Some("from-alice") => (Some(bob), Some(open)),
            other => panic!("{be}: unexpected hostname {other:?}"),
        };
        assert_eq!((row.owner_user_id, assigned), expected, "{be}");
    })
    .await;
}

/// Names that match nothing are skipped; the sysinfo is stored all the same.
#[tokio::test]
async fn unknown_preset_names_are_ignored() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let device_id = c.device(ID).await;
        c.accept_presets("all").await;

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
        c.accept_presets("row").await;
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
