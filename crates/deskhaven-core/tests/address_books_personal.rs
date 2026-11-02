//! AddressBookService, personal books: creation on first access (also concurrently),
//! privacy, peer writes, `hash` / `password` rules, `extra` round trip, capacity. Every test
//! runs on SQLite and PostgreSQL.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::ab::{bare, env, is_not_found, page, peer};
use deskhaven_core::addressbooks::{Actor, AddressBookError, Kind, PeerSecret};
use deskhaven_db::entity::{address_book, address_book_peer};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
use serde_json::{Value, json};

#[tokio::test]
async fn personal_book_is_created_once_on_first_access() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;

        let first = env.books.personal(alice).await.unwrap();
        let again = env.books.personal(alice).await.unwrap();

        assert_eq!(first, again);
        assert_eq!(first.kind, Kind::Personal);
        assert_eq!(first.owner_user_id, Some(alice));
        assert_eq!(first.guid.len(), 36, "uuid string: {}", first.guid);
        let count = address_book::Entity::find().count(&db.conn).await.unwrap();
        assert_eq!(count, 1);
    })
    .await;
}

#[tokio::test]
async fn concurrent_first_accesses_create_one_personal_book() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;

        let tasks: Vec<_> = (0..8)
            .map(|_| {
                let books = env.books.clone();
                tokio::spawn(async move { books.personal(alice).await })
            })
            .collect();
        let mut guids = Vec::new();
        for task in tasks {
            guids.push(task.await.unwrap().unwrap().guid);
        }

        guids.dedup();
        assert_eq!(guids.len(), 1, "{guids:?}");
        let count = address_book::Entity::find().count(&db.conn).await.unwrap();
        assert_eq!(count, 1);
    })
    .await;
}

#[tokio::test]
async fn personal_book_of_an_unknown_user_is_refused() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;

        let result = env.books.personal(4242).await;

        assert!(
            matches!(result, Err(AddressBookError::UserNotFound)),
            "{result:?}"
        );
    })
    .await;
}

#[tokio::test]
async fn personal_books_are_invisible_to_everyone_but_the_owner() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let bob = env.user("bob").await;
        let book = env.books.personal(alice).await.unwrap();
        env.books
            .add_peer(Actor::User(alice), &book.guid, bare("1"))
            .await
            .unwrap();

        for actor in [Actor::User(bob), Actor::Admin] {
            let guid = &book.guid;
            assert!(is_not_found(
                env.books.peers(actor, guid, page(1, 10)).await
            ));
            assert!(is_not_found(env.books.tags(actor, guid).await));
            assert!(is_not_found(
                env.books.add_peer(actor, guid, bare("2")).await
            ));
            assert!(is_not_found(
                env.books.delete_peers(actor, guid, &["1".into()]).await
            ));
        }
        assert_eq!(env.peer_ids(Actor::User(alice), &book.guid).await, ["1"]);
    })
    .await;
}

#[tokio::test]
async fn personal_books_keep_the_hash_and_never_a_password() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let book = env.books.personal(alice).await.unwrap();
        let me = Actor::User(alice);

        let sent = json!({ "id": "1", "hash": "h1", "password": "plain-pw" });
        env.books
            .add_peer(me, &book.guid, peer(sent))
            .await
            .unwrap();
        let update = json!({ "id": "1", "password": "plain-pw-2" });
        env.books
            .update_peer(me, &book.guid, peer(update))
            .await
            .unwrap();

        let row = address_book_peer::Entity::find()
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        let sealed = row.hash_enc.clone().unwrap();
        // A substring check against a 2-byte plaintext would hit random base64 ~1% of the
        // time; opening it under the row's context below is what proves it is sealed.
        assert!(sealed.starts_with("v1:"), "{sealed}");
        let context = format!("address_book_peers.hash_enc:{}:1", book.id);
        assert_eq!(
            env.crypto.open(&context, &sealed).unwrap().as_slice(),
            b"h1"
        );
        assert_eq!(row.password_enc, None);
        assert!(!row.extra.contains("plain-pw"), "{}", row.extra);
        let listed = env.books.peers(me, &book.guid, page(1, 10)).await.unwrap();
        assert_eq!(listed.items[0].secret, PeerSecret::Hash("h1".into()));
        let out = Value::Object(listed.items[0].to_json());
        assert_eq!(out["hash"], json!("h1"));
        assert!(out.get("password").is_none(), "{out}");
    })
    .await;
}

#[tokio::test]
async fn personal_hash_moved_to_another_row_does_not_open_and_empty_clears() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let book = env.books.personal(alice).await.unwrap();
        let me = Actor::User(alice);
        let sent = json!({ "id": "1", "hash": "salted-1" });
        env.books
            .add_peer(me, &book.guid, peer(sent))
            .await
            .unwrap();
        env.books.add_peer(me, &book.guid, bare("2")).await.unwrap();
        let find = async |id: &str| {
            address_book_peer::Entity::find_by_id((book.id, id.to_owned()))
                .one(&db.conn)
                .await
                .unwrap()
                .unwrap()
        };

        // An attacker with database write access copies peer 1's sealed hash to peer 2.
        let mut moved: address_book_peer::ActiveModel = find("2").await.into();
        moved.hash_enc = Set(find("1").await.hash_enc);
        moved.update(&db.conn).await.unwrap();
        let listed = env.books.peers(me, &book.guid, page(1, 10)).await.unwrap();
        let clear = json!({ "id": "1", "hash": "" });
        env.books
            .update_peer(me, &book.guid, peer(clear))
            .await
            .unwrap();

        let secrets: Vec<_> = listed.items.iter().map(|p| p.secret.clone()).collect();
        let hash = |h: &str| PeerSecret::Hash(h.to_owned());
        assert_eq!(secrets, [hash("salted-1"), hash("")]);
        assert_eq!(find("1").await.hash_enc, None);
    })
    .await;
}

#[tokio::test]
async fn peer_fields_and_unknown_extra_round_trip() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let book = env.books.personal(alice).await.unwrap();
        let me = Actor::User(alice);
        let sent = json!({
            "id": "123456789", "hash": "", "username": "u", "hostname": "h",
            "platform": "Windows", "alias": "office", "tags": ["work"],
            "forceAlwaysRelay": "true", "rdpPort": "3390", "rdpUsername": "rdp",
            "loginName": "login", "note": "first\nsecond", "device_group_name": "ignored",
            "same_server": true, "futureField": { "nested": [1, "two"] }
        });

        env.books
            .add_peer(me, &book.guid, peer(sent))
            .await
            .unwrap();
        let update = json!({ "id": "123456789", "alias": "home", "laterField": 7 });
        env.books
            .update_peer(me, &book.guid, peer(update))
            .await
            .unwrap();

        let listed = env.books.peers(me, &book.guid, page(1, 10)).await.unwrap();
        let out = Value::Object(listed.items[0].to_json());
        let expected = json!({
            "id": "123456789", "hash": "", "username": "u", "hostname": "h",
            "platform": "Windows", "alias": "home", "tags": ["work"],
            "forceAlwaysRelay": "true", "rdpPort": "3390", "rdpUsername": "rdp",
            "loginName": "login", "note": "first\nsecond",
            "futureField": { "nested": [1, "two"] }, "laterField": 7
        });
        assert_eq!(out, expected);
    })
    .await;
}

#[tokio::test]
async fn force_always_relay_is_stored_as_a_boolean() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let book = env.books.personal(alice).await.unwrap();
        let me = Actor::User(alice);

        let sent = json!({ "id": "1", "forceAlwaysRelay": "true" });
        env.books
            .add_peer(me, &book.guid, peer(sent))
            .await
            .unwrap();
        let stored = address_book_peer::Entity::find()
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        let update = json!({ "id": "1", "forceAlwaysRelay": "false" });
        env.books
            .update_peer(me, &book.guid, peer(update))
            .await
            .unwrap();
        let updated = address_book_peer::Entity::find()
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();

        assert!(stored.force_always_relay);
        assert!(!updated.force_always_relay);
        let listed = env.books.peers(me, &book.guid, page(1, 10)).await.unwrap();
        assert_eq!(
            Value::Object(listed.items[0].to_json())["forceAlwaysRelay"],
            json!("false")
        );
    })
    .await;
}

#[tokio::test]
async fn duplicate_missing_and_deleted_peers() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let book = env.books.personal(alice).await.unwrap();
        let me = Actor::User(alice);
        env.books.add_peer(me, &book.guid, bare("1")).await.unwrap();
        env.books.add_peer(me, &book.guid, bare("2")).await.unwrap();

        let duplicate = env.books.add_peer(me, &book.guid, bare("1")).await;
        let missing = env.books.update_peer(me, &book.guid, bare("3")).await;
        let ids = ["1".to_owned(), "nope".to_owned()];
        env.books.delete_peers(me, &book.guid, &ids).await.unwrap();

        assert!(
            matches!(duplicate, Err(AddressBookError::PeerExists)),
            "{duplicate:?}"
        );
        assert!(
            matches!(missing, Err(AddressBookError::PeerNotFound)),
            "{missing:?}"
        );
        assert_eq!(env.peer_ids(me, &book.guid).await, ["2"]);
    })
    .await;
}

#[tokio::test]
async fn capacity_follows_the_max_peers_setting() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let book = env.books.personal(alice).await.unwrap();
        let me = Actor::User(alice);
        assert_eq!(env.books.max_peers(), 1000);
        common::set(&env.settings, "addressbook.max_peers", json!(2)).await;

        env.books.add_peer(me, &book.guid, bare("1")).await.unwrap();
        env.books.add_peer(me, &book.guid, bare("2")).await.unwrap();
        let third = env.books.add_peer(me, &book.guid, bare("3")).await;

        assert_eq!(env.books.max_peers(), 2);
        assert!(
            matches!(third, Err(AddressBookError::Full { max: 2 })),
            "{third:?}"
        );
        // Updates and deletes still work on a full book.
        env.books
            .update_peer(me, &book.guid, peer(json!({"id": "2", "alias": "a"})))
            .await
            .unwrap();
        env.books
            .delete_peers(me, &book.guid, &["1".into()])
            .await
            .unwrap();
        env.books.add_peer(me, &book.guid, bare("3")).await.unwrap();
        let count = address_book_peer::Entity::find()
            .filter(address_book_peer::Column::AddressBookId.eq(book.id))
            .count(&db.conn)
            .await
            .unwrap();
        assert_eq!(count, 2);
    })
    .await;
}

#[tokio::test]
async fn peers_are_paged_by_id_with_the_total() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let book = env.books.personal(alice).await.unwrap();
        let me = Actor::User(alice);
        for id in ["c", "a", "b"] {
            env.books.add_peer(me, &book.guid, bare(id)).await.unwrap();
        }

        let first = env.books.peers(me, &book.guid, page(1, 2)).await.unwrap();
        let second = env.books.peers(me, &book.guid, page(2, 2)).await.unwrap();
        let bad = env.books.peers(me, &book.guid, page(0, 2)).await;

        assert_eq!(first.total, 3);
        let ids = |p: &[deskhaven_core::addressbooks::Peer]| -> Vec<String> {
            p.iter().map(|p| p.id.clone()).collect()
        };
        assert_eq!(ids(&first.items), ["a", "b"]);
        assert_eq!(ids(&second.items), ["c"]);
        assert!(
            matches!(bad, Err(AddressBookError::Invalid { field: "page", .. })),
            "{bad:?}"
        );
    })
    .await;
}
