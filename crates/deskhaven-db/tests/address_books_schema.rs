//! Address book schema (M2-02): the personal-book partial unique index, composite keys,
//! CHECKs and cascades, on every available backend.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used)]

mod common;

use common::ops::{
    address_book_row, assert_checks, assert_indexes, is_fk_violation, is_unique_violation,
    peer_row, share_row, tag_row,
};
use common::{count, user_row};
use deskhaven_db::entity::address_book::Kind;
use deskhaven_db::entity::address_book_share::{Rule, SubjectType};
use deskhaven_db::entity::{
    address_book, address_book_peer, address_book_share, address_book_tag, user,
};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{ActiveModelTrait, EntityTrait};
use sea_orm_migration::prelude::*;

const TABLES: [&str; 4] = [
    "address_books",
    "address_book_shares",
    "address_book_peers",
    "address_book_tags",
];

#[tokio::test]
async fn tables_and_indexes_exist() {
    each_migrated_backend(async |db: &TestDb| {
        let manager = SchemaManager::new(&db.conn);
        for table in TABLES {
            assert!(
                manager.has_table(table).await.unwrap(),
                "{}: missing {table}",
                db.backend
            );
        }
        assert_indexes(
            db,
            &[
                ("address_books", "uq_address_books_guid"),
                ("address_books", "uq_address_books_personal_owner"),
                ("address_books", "idx_address_books_owner_user_id"),
                ("address_book_shares", "idx_address_book_shares_subject"),
            ],
        )
        .await;
    })
    .await;
}

#[tokio::test]
async fn guid_is_unique() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let alice = user_row("alice").insert(conn).await.unwrap();
        address_book_row(alice.id, Kind::Shared, "g1")
            .insert(conn)
            .await
            .unwrap();

        let dup = address_book_row(alice.id, Kind::Shared, "g1")
            .insert(conn)
            .await;

        assert!(is_unique_violation(dup), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn a_user_has_at_most_one_personal_book() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let alice = user_row("alice").insert(conn).await.unwrap();
        let bob = user_row("bob").insert(conn).await.unwrap();
        address_book_row(alice.id, Kind::Personal, "p-alice")
            .insert(conn)
            .await
            .unwrap();

        let second = address_book_row(alice.id, Kind::Personal, "p-alice-2")
            .insert(conn)
            .await;
        let other_user = address_book_row(bob.id, Kind::Personal, "p-bob")
            .insert(conn)
            .await;

        let b = db.backend;
        assert!(is_unique_violation(second), "{b}: second personal book");
        assert!(other_user.is_ok(), "{b}: another user's personal book");
    })
    .await;
}

#[tokio::test]
async fn shared_books_are_not_limited_by_the_personal_index() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let alice = user_row("alice").insert(conn).await.unwrap();
        address_book_row(alice.id, Kind::Personal, "p")
            .insert(conn)
            .await
            .unwrap();

        for guid in ["s1", "s2", "s3"] {
            let shared = address_book_row(alice.id, Kind::Shared, guid)
                .insert(conn)
                .await;
            assert!(shared.is_ok(), "{}: {guid}", db.backend);
        }

        assert_eq!(count::<address_book::Entity>(conn).await, 4);
    })
    .await;
}

#[tokio::test]
async fn a_shared_book_may_not_be_turned_into_a_second_personal_one() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let alice = user_row("alice").insert(conn).await.unwrap();
        address_book_row(alice.id, Kind::Personal, "p")
            .insert(conn)
            .await
            .unwrap();
        address_book_row(alice.id, Kind::Shared, "s")
            .insert(conn)
            .await
            .unwrap();

        assert_checks(
            db,
            &[(
                "address_books",
                "kind = 'personal' WHERE guid = 's'",
                "kind = 'shared' WHERE guid = 's'",
            )],
        )
        .await;
    })
    .await;
}

#[tokio::test]
async fn book_kind_is_checked() {
    each_migrated_backend(async |db: &TestDb| {
        let alice = user_row("alice").insert(&db.conn).await.unwrap();
        address_book_row(alice.id, Kind::Shared, "s")
            .insert(&db.conn)
            .await
            .unwrap();

        assert_checks(db, &[("address_books", "kind = 'team'", "kind = 'shared'")]).await;
    })
    .await;
}

#[tokio::test]
async fn books_reject_an_unknown_owner() {
    each_migrated_backend(async |db: &TestDb| {
        let res = address_book_row(404, Kind::Shared, "s")
            .insert(&db.conn)
            .await;

        assert!(is_fk_violation(res), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn children_reject_an_unknown_book() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;

        let share = share_row(404, SubjectType::User, 1, Rule::ReadOnly)
            .insert(conn)
            .await;
        let peer = peer_row(404, "1").insert(conn).await;
        let tag = tag_row(404, "red").insert(conn).await;

        let b = db.backend;
        assert!(is_fk_violation(share), "{b}: share");
        assert!(is_fk_violation(peer), "{b}: peer");
        assert!(is_fk_violation(tag), "{b}: tag");
    })
    .await;
}

#[tokio::test]
async fn deleting_a_book_removes_its_children_only() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let alice = user_row("alice").insert(conn).await.unwrap();
        let gone = address_book_row(alice.id, Kind::Shared, "gone")
            .insert(conn)
            .await
            .unwrap();
        let kept = address_book_row(alice.id, Kind::Shared, "kept")
            .insert(conn)
            .await
            .unwrap();
        for book in [&gone, &kept] {
            share_row(book.id, SubjectType::Group, 9, Rule::ReadWrite)
                .insert(conn)
                .await
                .unwrap();
            peer_row(book.id, "1").insert(conn).await.unwrap();
            tag_row(book.id, "red").insert(conn).await.unwrap();
        }

        address_book::Entity::delete_by_id(gone.id)
            .exec(conn)
            .await
            .unwrap();

        let b = db.backend;
        assert_eq!(
            count::<address_book_share::Entity>(conn).await,
            1,
            "{b}: shares"
        );
        assert_eq!(
            count::<address_book_peer::Entity>(conn).await,
            1,
            "{b}: peers"
        );
        assert_eq!(
            count::<address_book_tag::Entity>(conn).await,
            1,
            "{b}: tags"
        );
        assert_eq!(count::<user::Entity>(conn).await, 1, "{b}: owner kept");
    })
    .await;
}

#[tokio::test]
async fn deleting_a_user_removes_their_books() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let alice = user_row("alice").insert(conn).await.unwrap();
        let bob = user_row("bob").insert(conn).await.unwrap();
        let personal = address_book_row(alice.id, Kind::Personal, "p")
            .insert(conn)
            .await
            .unwrap();
        address_book_row(alice.id, Kind::Shared, "s")
            .insert(conn)
            .await
            .unwrap();
        address_book_row(bob.id, Kind::Personal, "p-bob")
            .insert(conn)
            .await
            .unwrap();
        peer_row(personal.id, "1").insert(conn).await.unwrap();

        user::Entity::delete_by_id(alice.id)
            .exec(conn)
            .await
            .unwrap();

        let b = db.backend;
        assert_eq!(count::<address_book::Entity>(conn).await, 1, "{b}: books");
        assert_eq!(
            count::<address_book_peer::Entity>(conn).await,
            0,
            "{b}: peers"
        );
    })
    .await;
}

#[tokio::test]
async fn share_key_is_book_subject_type_and_id() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let alice = user_row("alice").insert(conn).await.unwrap();
        let book = address_book_row(alice.id, Kind::Shared, "s")
            .insert(conn)
            .await
            .unwrap();
        share_row(book.id, SubjectType::User, 5, Rule::ReadOnly)
            .insert(conn)
            .await
            .unwrap();

        let dup = share_row(book.id, SubjectType::User, 5, Rule::FullControl)
            .insert(conn)
            .await;
        // Same numeric id, other subject type: a different subject.
        let group = share_row(book.id, SubjectType::Group, 5, Rule::ReadOnly)
            .insert(conn)
            .await;

        assert!(is_unique_violation(dup), "{}: duplicate share", db.backend);
        assert!(group.is_ok(), "{}: group 5 is not user 5", db.backend);
    })
    .await;
}

#[tokio::test]
async fn share_columns_are_checked() {
    each_migrated_backend(async |db: &TestDb| {
        let alice = user_row("alice").insert(&db.conn).await.unwrap();
        let book = address_book_row(alice.id, Kind::Shared, "s")
            .insert(&db.conn)
            .await
            .unwrap();
        share_row(book.id, SubjectType::User, 5, Rule::ReadOnly)
            .insert(&db.conn)
            .await
            .unwrap();

        assert_checks(
            db,
            &[
                ("address_book_shares", "rule = 0", "rule = 2"),
                ("address_book_shares", "rule = 4", "rule = 3"),
                (
                    "address_book_shares",
                    "subject_type = 'everyone'",
                    "subject_type = 'group'",
                ),
            ],
        )
        .await;
    })
    .await;
}

#[tokio::test]
async fn share_rules_round_trip_as_numbers() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let alice = user_row("alice").insert(conn).await.unwrap();
        let book = address_book_row(alice.id, Kind::Shared, "s")
            .insert(conn)
            .await
            .unwrap();

        for (subject, rule) in [
            (1, Rule::ReadOnly),
            (2, Rule::ReadWrite),
            (3, Rule::FullControl),
        ] {
            let row = share_row(book.id, SubjectType::User, subject, rule)
                .insert(conn)
                .await
                .unwrap();
            assert_eq!(row.rule, rule, "{}", db.backend);
        }

        let stored: Vec<_> = address_book_share::Entity::find()
            .all(conn)
            .await
            .unwrap()
            .into_iter()
            .map(|s| (s.subject_id, s.rule))
            .collect();
        assert_eq!(
            stored,
            vec![
                (1, Rule::ReadOnly),
                (2, Rule::ReadWrite),
                (3, Rule::FullControl)
            ],
            "{}",
            db.backend
        );
    })
    .await;
}

#[tokio::test]
async fn peer_key_is_book_and_rustdesk_id() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let alice = user_row("alice").insert(conn).await.unwrap();
        let one = address_book_row(alice.id, Kind::Shared, "one")
            .insert(conn)
            .await
            .unwrap();
        let two = address_book_row(alice.id, Kind::Shared, "two")
            .insert(conn)
            .await
            .unwrap();
        peer_row(one.id, "123").insert(conn).await.unwrap();

        let dup = peer_row(one.id, "123").insert(conn).await;
        let other_book = peer_row(two.id, "123").insert(conn).await;

        assert!(is_unique_violation(dup), "{}: duplicate peer", db.backend);
        assert!(
            other_book.is_ok(),
            "{}: same device, other book",
            db.backend
        );
    })
    .await;
}

#[tokio::test]
async fn peer_round_trips_with_json_and_credentials() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let alice = user_row("alice").insert(conn).await.unwrap();
        let book = address_book_row(alice.id, Kind::Personal, "p")
            .insert(conn)
            .await
            .unwrap();
        let row = address_book_peer::ActiveModel {
            hash: sea_orm::Set(Some("client-hash".to_owned())),
            force_always_relay: sea_orm::Set(true),
            extra: sea_orm::Set(r#"{"future-field":[1,2]}"#.to_owned()),
            ..peer_row(book.id, "123")
        };

        let inserted = row.insert(conn).await.unwrap();

        let found = address_book_peer::Entity::find_by_id((book.id, "123".to_owned()))
            .one(conn)
            .await
            .unwrap();
        assert_eq!(found, Some(inserted), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn peer_boolean_is_checked() {
    each_migrated_backend(async |db: &TestDb| {
        let alice = user_row("alice").insert(&db.conn).await.unwrap();
        let book = address_book_row(alice.id, Kind::Shared, "s")
            .insert(&db.conn)
            .await
            .unwrap();
        peer_row(book.id, "1").insert(&db.conn).await.unwrap();

        assert_checks(
            db,
            &[(
                "address_book_peers",
                "force_always_relay = 2",
                "force_always_relay = true",
            )],
        )
        .await;
    })
    .await;
}

#[tokio::test]
async fn tag_key_is_book_and_name_and_color_holds_argb() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let alice = user_row("alice").insert(conn).await.unwrap();
        let one = address_book_row(alice.id, Kind::Shared, "one")
            .insert(conn)
            .await
            .unwrap();
        let two = address_book_row(alice.id, Kind::Shared, "two")
            .insert(conn)
            .await
            .unwrap();
        let tag = tag_row(one.id, "red").insert(conn).await.unwrap();

        let dup = tag_row(one.id, "red").insert(conn).await;
        let other_book = tag_row(two.id, "red").insert(conn).await;

        let b = db.backend;
        assert_eq!(tag.color, 0xff12_34ab, "{b}: ARGB above i32::MAX survives");
        assert!(is_unique_violation(dup), "{b}: duplicate tag");
        assert!(other_book.is_ok(), "{b}: same name, other book");
    })
    .await;
}
