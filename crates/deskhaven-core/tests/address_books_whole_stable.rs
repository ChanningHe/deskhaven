//! AddressBookService whole-book operations, part 2: a push that changes nothing writes
//! nothing (timestamps and sealed secrets stay), a push that changes one peer rewrites only
//! that peer, and a read is one snapshot even while pushes run. Every test runs on SQLite
//! and PostgreSQL.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use common::ab::{env, peer};
use deskhaven_core::addressbooks::{Actor, BookReplacement, ReplacementTag};
use deskhaven_db::entity::address_book_peer;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::EntityTrait;
use serde_json::{Value, json};

/// Longer than the millisecond resolution of the stored timestamps.
const TICK: Duration = Duration::from_millis(5);

fn push(peers: Vec<Value>, tags: &[(&str, i64)]) -> BookReplacement {
    BookReplacement {
        peers: peers.into_iter().map(peer).collect(),
        tags: tags
            .iter()
            .map(|(name, color)| ReplacementTag {
                name: (*name).to_owned(),
                color: Some(*color),
            })
            .collect(),
    }
}

/// `rustdesk_id -> (updated_at ms, sealed hash)` of every peer row.
async fn stamps(db: &TestDb) -> BTreeMap<String, (i64, Option<String>)> {
    let rows = address_book_peer::Entity::find()
        .all(&db.conn)
        .await
        .unwrap();
    rows.into_iter()
        .map(|r| {
            let at = r.updated_at.timestamp_millis();
            (r.rustdesk_id, (at, r.hash_enc))
        })
        .collect()
}

fn document() -> Vec<Value> {
    vec![
        json!({ "id": "1", "alias": "a", "hash": "h1", "tags": ["t"], "futureField": [1] }),
        json!({ "id": "2", "alias": "b", "hash": "h2", "note": "n" }),
    ]
}

#[tokio::test]
async fn an_identical_push_writes_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let me = Actor::User(alice);
        let book = env.books.personal(alice).await.unwrap();
        let first = push(document(), &[("t", 5)]);
        env.books
            .replace_contents(me, &book.guid, first)
            .await
            .unwrap();
        let stamps_before = stamps(db).await;
        let book_before = env.books.personal(alice).await.unwrap();
        tokio::time::sleep(TICK).await;

        let again = push(document(), &[("t", 5)]);
        env.books
            .replace_contents(me, &book.guid, again)
            .await
            .unwrap();

        assert_eq!(
            stamps(db).await,
            stamps_before,
            "rows, timestamps and ciphertexts"
        );
        let book_after = env.books.personal(alice).await.unwrap();
        assert_eq!(book_after.updated_at, book_before.updated_at);
    })
    .await;
}

#[tokio::test]
async fn changing_one_peer_rewrites_only_that_peer() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let me = Actor::User(alice);
        let book = env.books.personal(alice).await.unwrap();
        let first = push(document(), &[("t", 5)]);
        env.books
            .replace_contents(me, &book.guid, first)
            .await
            .unwrap();
        let before = stamps(db).await;
        tokio::time::sleep(TICK).await;

        let mut changed = document();
        changed[1]["alias"] = json!("renamed");
        let second = push(changed, &[("t", 5)]);
        env.books
            .replace_contents(me, &book.guid, second)
            .await
            .unwrap();

        let after = stamps(db).await;
        assert_eq!(after["1"], before["1"], "the unchanged peer is not touched");
        assert!(after["2"].0 > before["2"].0, "the changed peer is");
        let contents = env.books.contents(me, &book.guid).await.unwrap();
        assert_eq!(contents.peers[1].alias, "renamed");
        assert_eq!(contents.peers[1].note, "n");
        let book_after = env.books.personal(alice).await.unwrap();
        assert!(book_after.updated_at.0 > book.updated_at.0);
    })
    .await;
}

#[tokio::test]
async fn a_changed_tag_colour_or_a_removed_peer_is_still_written() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let me = Actor::User(alice);
        let book = env.books.personal(alice).await.unwrap();
        let first = push(document(), &[("t", 5)]);
        env.books
            .replace_contents(me, &book.guid, first)
            .await
            .unwrap();

        let recoloured = push(document(), &[("t", 6)]);
        env.books
            .replace_contents(me, &book.guid, recoloured)
            .await
            .unwrap();
        let tags = env.books.tags(me, &book.guid).await.unwrap();
        let fewer = push(document()[..1].to_vec(), &[("t", 6)]);
        env.books
            .replace_contents(me, &book.guid, fewer)
            .await
            .unwrap();

        assert_eq!(tags[0].color, 6);
        assert_eq!(env.peer_ids(me, &book.guid).await, ["1"]);
    })
    .await;
}

/// Writers flip the book between two documents whose peers and tags carry the same marker;
/// every concurrent read must be wholly one of them (peers and tags from one snapshot).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_read_is_one_snapshot_while_pushes_run() {
    each_migrated_backend(async |db: &TestDb| {
        const FLIPS: usize = 25;
        const READS: usize = 60;
        let env = env(db).await;
        let alice = env.user("alice").await;
        let book = env.books.personal(alice).await.unwrap();
        let doc = |marker: &str| {
            let peers = (0..20).map(|i| json!({ "id": format!("{marker}{i}"), "alias": marker }));
            push(peers.collect(), &[(marker, 1), (&format!("{marker}2"), 2)])
        };
        let me = Actor::User(alice);
        env.books
            .replace_contents(me, &book.guid, doc("a"))
            .await
            .unwrap();
        let writer = {
            let (books, guid) = (env.books.clone(), book.guid.clone());
            tokio::spawn(async move {
                for i in 0..FLIPS {
                    let marker = if i % 2 == 0 { "b" } else { "a" };
                    books
                        .replace_contents(me, &guid, doc(marker))
                        .await
                        .unwrap();
                }
            })
        };
        let reader = {
            let (books, guid) = (env.books.clone(), book.guid.clone());
            tokio::spawn(async move {
                let mut seen = Vec::new();
                for _ in 0..READS {
                    seen.push(books.contents(me, &guid).await.unwrap());
                }
                seen
            })
        };

        writer.await.unwrap();
        for contents in reader.await.unwrap() {
            let markers: std::collections::BTreeSet<_> = contents
                .peers
                .iter()
                .map(|p| p.alias.clone())
                .chain(contents.tags.iter().map(|t| t.name[..1].to_owned()))
                .collect();
            assert_eq!(markers.len(), 1, "{}: torn read {markers:?}", db.backend);
            assert_eq!(contents.peers.len(), 20);
        }
    })
    .await;
}
