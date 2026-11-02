//! AddressBookService, shared books: passwords sealed at rest (and bound to their row),
//! no `hash`, name rules, tag rename / delete rewriting peers. Every test runs on SQLite and
//! PostgreSQL.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::ab::{bare, env, page, peer};
use deskhaven_core::addressbooks::{
    Actor, AddressBookError, Kind, NewSharedBook, PeerSecret, Rule, SharedBookUpdate, Tag,
};
use deskhaven_db::entity::{address_book, address_book_peer};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};
use zeroize::Zeroizing;

fn password(text: &str) -> PeerSecret {
    PeerSecret::Password(Zeroizing::new(text.to_owned()))
}

async fn row(db: &TestDb, id: &str) -> address_book_peer::Model {
    address_book_peer::Entity::find()
        .filter(address_book_peer::Column::RustdeskId.eq(id))
        .one(&db.conn)
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn shared_passwords_are_sealed_at_rest_and_never_store_a_hash() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let book = env.shared_with("team", alice, Rule::ReadWrite).await;
        let me = Actor::User(alice);

        let sent = json!({ "id": "1", "password": "s3cret-pw", "hash": "client-hash" });
        env.books
            .add_peer(me, &book.guid, peer(sent))
            .await
            .unwrap();

        let stored = row(db, "1").await;
        let sealed = stored.password_enc.clone().unwrap();
        assert!(sealed.starts_with("v1:"), "{sealed}");
        assert!(!sealed.contains("s3cret-pw"));
        assert_eq!(stored.hash, None);
        assert!(!stored.extra.contains("client-hash"), "{}", stored.extra);
        let context = format!("address_book_peers.password_enc:{}:1", book.id);
        let opened = env.crypto.open(&context, &sealed).unwrap();
        assert_eq!(opened.as_slice(), b"s3cret-pw");
        let listed = env.books.peers(me, &book.guid, page(1, 10)).await.unwrap();
        assert_eq!(listed.items[0].secret, password("s3cret-pw"));
        let out = Value::Object(listed.items[0].to_json());
        assert_eq!(out["password"], json!("s3cret-pw"));
        assert!(out.get("hash").is_none(), "{out}");
    })
    .await;
}

#[tokio::test]
async fn shared_password_updates_reseal_and_an_empty_one_clears() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let book = env.shared_with("team", alice, Rule::ReadWrite).await;
        let me = Actor::User(alice);
        env.books
            .add_peer(me, &book.guid, peer(json!({"id": "1", "password": "a"})))
            .await
            .unwrap();
        let before = row(db, "1").await.password_enc;

        let change = json!({ "id": "1", "password": "b" });
        env.books
            .update_peer(me, &book.guid, peer(change))
            .await
            .unwrap();
        let changed = row(db, "1").await.password_enc;
        let alias_only = json!({ "id": "1", "alias": "x" });
        env.books
            .update_peer(me, &book.guid, peer(alias_only))
            .await
            .unwrap();
        let kept = row(db, "1").await.password_enc;
        let clear = json!({ "id": "1", "password": "" });
        env.books
            .update_peer(me, &book.guid, peer(clear))
            .await
            .unwrap();

        assert_ne!(before, changed);
        assert_eq!(changed, kept);
        assert_eq!(row(db, "1").await.password_enc, None);
        let listed = env.books.peers(me, &book.guid, page(1, 10)).await.unwrap();
        assert_eq!(listed.items[0].secret, password(""));
    })
    .await;
}

#[tokio::test]
async fn a_password_moved_to_another_row_does_not_open() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let book = env.shared_with("team", alice, Rule::ReadWrite).await;
        let me = Actor::User(alice);
        env.books
            .add_peer(me, &book.guid, peer(json!({"id": "1", "password": "pw1"})))
            .await
            .unwrap();
        env.books.add_peer(me, &book.guid, bare("2")).await.unwrap();

        // An attacker with database write access copies the ciphertext to peer 2.
        let mut moved: address_book_peer::ActiveModel = row(db, "2").await.into();
        moved.password_enc = Set(row(db, "1").await.password_enc);
        moved.update(&db.conn).await.unwrap();

        let listed = env.books.peers(me, &book.guid, page(1, 10)).await.unwrap();
        let secrets: Vec<_> = listed
            .items
            .iter()
            .map(|p| (p.id.as_str(), &p.secret))
            .collect();
        assert_eq!(secrets, [("1", &password("pw1")), ("2", &password(""))]);
    })
    .await;
}

#[tokio::test]
async fn shared_names_are_unique_case_insensitively_and_not_reserved() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let book = env.shared("Team", None).await;
        let other = env.shared("Ops", None).await;
        let new = |name: &str| NewSharedBook {
            name: name.to_owned(),
            note: String::new(),
            owner_user_id: None,
        };

        for name in [" team ", "TEAM", "My address book", "legacy ADDRESS book"] {
            let result = env.books.create_shared(new(name)).await;
            assert!(
                matches!(result, Err(AddressBookError::NameTaken)),
                "{name}: {result:?}"
            );
        }
        let blank = env.books.create_shared(new("  ")).await;
        assert!(matches!(
            blank,
            Err(AddressBookError::Invalid { field: "name", .. })
        ));
        let rename = |name: &str| SharedBookUpdate {
            name: Some(name.to_owned()),
            ..SharedBookUpdate::default()
        };
        let clash = env
            .books
            .update_shared(Actor::Admin, &other.guid, rename("team"))
            .await;
        assert!(
            matches!(clash, Err(AddressBookError::NameTaken)),
            "{clash:?}"
        );
        let same = env
            .books
            .update_shared(Actor::Admin, &book.guid, rename("TEAM"))
            .await;
        assert_eq!(same.unwrap().name, "TEAM");
        assert_eq!(
            env.books.find_shared_by_name(" team ").await.unwrap().guid,
            book.guid
        );
        let missing = env.books.find_shared_by_name("nope").await;
        assert!(
            matches!(missing, Err(AddressBookError::NotFound)),
            "{missing:?}"
        );
    })
    .await;
}

#[tokio::test]
async fn resolving_a_duplicated_name_refuses_to_guess() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        env.shared("Team", None).await;
        // The schema allows it (e.g. a PostgreSQL race between two creations).
        let now = time::now();
        address_book::ActiveModel {
            guid: Set("dup-guid".to_owned()),
            kind: Set(Kind::Shared),
            owner_user_id: Set(None),
            name: Set("team".to_owned()),
            note: Set(String::new()),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(&db.conn)
        .await
        .unwrap();

        let result = env.books.find_shared_by_name("Team").await;

        assert!(
            matches!(result, Err(AddressBookError::AmbiguousName)),
            "{result:?}"
        );
    })
    .await;
}

#[tokio::test]
async fn personal_books_cannot_be_managed_as_shared() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let bob = env.user("bob").await;
        let book = env.books.personal(alice).await.unwrap();
        let me = Actor::User(alice);
        let subject = deskhaven_core::addressbooks::Subject::User(bob);

        let share = env
            .books
            .set_share(me, &book.guid, subject, Rule::ReadOnly)
            .await;
        let delete = env.books.delete_shared(me, &book.guid).await;
        let update = env
            .books
            .update_shared(me, &book.guid, SharedBookUpdate::default())
            .await;

        assert!(
            matches!(share, Err(AddressBookError::Personal)),
            "{share:?}"
        );
        assert!(
            matches!(delete, Err(AddressBookError::Personal)),
            "{delete:?}"
        );
        assert!(
            matches!(update, Err(AddressBookError::Personal)),
            "{update:?}"
        );
    })
    .await;
}

#[tokio::test]
async fn renaming_and_deleting_tags_rewrites_peer_tags() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let book = env.books.personal(alice).await.unwrap();
        let me = Actor::User(alice);
        let tag = |name: &str, color: i64| Tag {
            name: name.to_owned(),
            color,
        };
        for t in [tag("a", 1), tag("b", 2), tag("c", 0xFFFF_FFFF)] {
            env.books.add_tag(me, &book.guid, t).await.unwrap();
        }
        let sent = json!({ "id": "1", "tags": ["a", "b", "c"] });
        env.books
            .add_peer(me, &book.guid, peer(sent))
            .await
            .unwrap();
        env.books
            .add_peer(me, &book.guid, peer(json!({"id": "2", "tags": ["b"]})))
            .await
            .unwrap();

        env.books
            .add_tag(me, &book.guid, tag("a", 99))
            .await
            .unwrap();
        env.books
            .rename_tag(me, &book.guid, "a", "z")
            .await
            .unwrap();
        env.books
            .delete_tags(me, &book.guid, &["b".into(), "nope".into()])
            .await
            .unwrap();
        env.books
            .update_tag(me, &book.guid, tag("c", 3))
            .await
            .unwrap();

        let tags = env.books.tags(me, &book.guid).await.unwrap();
        assert_eq!(tags, [tag("c", 3), tag("z", 1)]);
        let peers = env.books.peers(me, &book.guid, page(1, 10)).await.unwrap();
        let lists: Vec<_> = peers.items.iter().map(|p| p.tags.clone()).collect();
        assert_eq!(lists, [vec!["z".to_owned(), "c".to_owned()], vec![]]);
        let exists = env.books.rename_tag(me, &book.guid, "c", "z").await;
        let missing = env.books.rename_tag(me, &book.guid, "nope", "y").await;
        let update_missing = env.books.update_tag(me, &book.guid, tag("nope", 1)).await;
        let bad_color = env
            .books
            .add_tag(me, &book.guid, tag("d", 0x1_0000_0000))
            .await;
        assert!(
            matches!(exists, Err(AddressBookError::TagExists)),
            "{exists:?}"
        );
        assert!(
            matches!(missing, Err(AddressBookError::TagNotFound)),
            "{missing:?}"
        );
        assert!(matches!(update_missing, Err(AddressBookError::TagNotFound)));
        assert!(matches!(
            bad_color,
            Err(AddressBookError::Invalid { field: "color", .. })
        ));
    })
    .await;
}
