//! AddressBookService, request size limits: delete lists are capped (refused before any row
//! is touched) and deleting tags is linear in peers × tags, so a large request cannot hold
//! the write lock for long. Every test runs on SQLite and PostgreSQL.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::time::{Duration, Instant};

use common::ab::{bare, env, page};
use deskhaven_core::addressbooks::{Actor, AddressBookError, Tag};
use deskhaven_db::entity::{address_book_peer, address_book_tag};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;
use sea_orm::ActiveValue::Set;
use sea_orm::{EntityTrait, PaginatorTrait};
use serde_json::Value;

/// Most names `delete_tags` takes (the most tags one book may hold).
const MAX_TAG_NAMES: usize = 1000;
const FULL_BOOK_PEERS: usize = 1000;
const TAGS_PER_PEER: usize = 100;
const INSERT_CHUNK: usize = 200;
/// Far above what the linear algorithm needs even unoptimized on PostgreSQL; the quadratic
/// one took seconds of CPU per request at this size.
const GENEROUS: Duration = Duration::from_secs(20);

fn tag_names(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("t{i}")).collect()
}

#[tokio::test]
async fn oversized_delete_lists_are_refused_without_touching_rows() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let book = env.books.personal(alice).await.unwrap();
        let me = Actor::User(alice);
        env.books
            .add_peer(me, &book.guid, bare("t0"))
            .await
            .unwrap();
        let tag = Tag {
            name: "t0".to_owned(),
            color: 1,
        };
        env.books.add_tag(me, &book.guid, tag).await.unwrap();
        common::set(&env.settings, "addressbook.max_peers", serde_json::json!(5)).await;

        let ids = tag_names(6);
        let peers = env.books.delete_peers(me, &book.guid, &ids).await;
        let names = tag_names(MAX_TAG_NAMES + 1);
        let tags = env.books.delete_tags(me, &book.guid, &names).await;

        assert!(
            matches!(peers, Err(AddressBookError::Invalid { field: "ids", .. })),
            "{peers:?}"
        );
        assert!(
            matches!(tags, Err(AddressBookError::Invalid { field: "tags", .. })),
            "{tags:?}"
        );
        assert_eq!(env.peer_ids(me, &book.guid).await, ["t0"]);
        assert_eq!(env.books.tags(me, &book.guid).await.unwrap().len(), 1);
        // At the caps the lists are accepted.
        env.books
            .delete_peers(me, &book.guid, &tag_names(5))
            .await
            .unwrap();
        let at_cap = tag_names(MAX_TAG_NAMES);
        env.books
            .delete_tags(me, &book.guid, &at_cap)
            .await
            .unwrap();
        assert!(env.peer_ids(me, &book.guid).await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn deleting_every_tag_of_a_full_book_is_linear() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let book = env.books.personal(alice).await.unwrap();
        let me = Actor::User(alice);
        let names = tag_names(TAGS_PER_PEER);
        let tags_json = Value::from(names.clone()).to_string();
        let tag_rows = names.iter().map(|name| address_book_tag::ActiveModel {
            address_book_id: Set(book.id),
            name: Set(name.clone()),
            color: Set(1),
        });
        address_book_tag::Entity::insert_many(tag_rows)
            .exec(&db.conn)
            .await
            .unwrap();
        let ids: Vec<String> = (0..FULL_BOOK_PEERS).map(|i| format!("{i:09}")).collect();
        for chunk in ids.chunks(INSERT_CHUNK) {
            let rows = chunk.iter().map(|id| address_book_peer::ActiveModel {
                address_book_id: Set(book.id),
                rustdesk_id: Set(id.clone()),
                alias: Set(String::new()),
                note: Set(String::new()),
                tags: Set(tags_json.clone()),
                hash_enc: Set(None),
                password_enc: Set(None),
                username: Set(String::new()),
                hostname: Set(String::new()),
                platform: Set(String::new()),
                force_always_relay: Set(false),
                rdp_port: Set(String::new()),
                rdp_username: Set(String::new()),
                login_name: Set(String::new()),
                extra: Set("{}".to_owned()),
                updated_at: Set(time::now()),
            });
            address_book_peer::Entity::insert_many(rows)
                .exec(&db.conn)
                .await
                .unwrap();
        }
        // The largest accepted request: every tag, then duplicates up to the cap.
        let mut request = names.clone();
        request.extend(
            names
                .iter()
                .cycle()
                .take(MAX_TAG_NAMES - TAGS_PER_PEER)
                .cloned(),
        );

        let started = Instant::now();
        env.books
            .delete_tags(me, &book.guid, &request)
            .await
            .unwrap();
        let took = started.elapsed();

        assert!(took < GENEROUS, "{}: delete_tags took {took:?}", db.backend);
        let remaining = address_book_tag::Entity::find()
            .count(&db.conn)
            .await
            .unwrap();
        assert_eq!(remaining, 0);
        let listed = env.books.peers(me, &book.guid, page(1, 100)).await.unwrap();
        assert_eq!(listed.total, FULL_BOOK_PEERS as u64);
        assert!(listed.items.iter().all(|p| p.tags.is_empty()));
    })
    .await;
}
