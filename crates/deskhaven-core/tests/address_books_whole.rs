//! AddressBookService, whole-book operations (`contents`, `replace_contents`): what legacy
//! `/api/ab` clients read and push. Replacement is one transaction; fields the pushed peers
//! do not carry survive; the caps of the per-item API apply. Every test runs on SQLite and
//! PostgreSQL.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::ab::{env, is_forbidden, is_not_found, peer};
use deskhaven_core::addressbooks::{
    Actor, AddressBookError, BookReplacement, PeerSecret, ReplacementTag, Rule, Tag,
};
use deskhaven_db::entity::{address_book_peer, address_book_tag};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{EntityTrait, PaginatorTrait};
use serde_json::json;

/// Colour of a tag whose colour nobody gave (the client's accent blue).
const DEFAULT_COLOR: i64 = 0xFF00_71FF;

fn tag(name: &str, color: Option<i64>) -> ReplacementTag {
    ReplacementTag {
        name: name.to_owned(),
        color,
    }
}

fn replacement(peers: Vec<serde_json::Value>, tags: Vec<ReplacementTag>) -> BookReplacement {
    BookReplacement {
        peers: peers.into_iter().map(peer).collect(),
        tags,
    }
}

#[tokio::test]
async fn a_new_personal_book_has_no_contents() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let book = env.books.personal(alice).await.unwrap();

        let contents = env
            .books
            .contents(Actor::User(alice), &book.guid)
            .await
            .unwrap();

        assert!(contents.peers.is_empty() && contents.tags.is_empty());
        assert_eq!(contents.updated_at, book.updated_at);
    })
    .await;
}

#[tokio::test]
async fn replacing_stores_peers_and_tags_and_contents_reads_them_back() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let book = env.books.personal(alice).await.unwrap();
        let me = Actor::User(alice);
        let new = replacement(
            vec![
                json!({ "id": "222", "alias": "b", "hash": "h2", "password": "dropped", "tags": ["x"] }),
                json!({ "id": "111", "username": "u", "hostname": "h", "platform": "Linux",
                       "tags": ["x", "y"], "futureField": [1] }),
            ],
            vec![tag("x", Some(0xFF11_2233)), tag("y", Some(0xFF44_5566))],
        );

        env.books
            .replace_contents(me, &book.guid, new)
            .await
            .unwrap();
        let contents = env.books.contents(me, &book.guid).await.unwrap();

        let ids: Vec<_> = contents.peers.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["111", "222"], "ordered by id");
        let first = &contents.peers[0];
        assert_eq!(first.username, "u");
        assert_eq!(first.platform, "Linux");
        assert_eq!(first.tags, ["x", "y"]);
        assert_eq!(first.extra["futureField"], json!([1]));
        assert_eq!(
            contents.peers[1].secret,
            PeerSecret::Hash("h2".to_owned()),
            "the hash comes back; the password was never kept"
        );
        let tags: Vec<_> = contents.tags.iter().map(|t| (t.name.as_str(), t.color)).collect();
        assert_eq!(tags, [("x", 0xFF11_2233), ("y", 0xFF44_5566)]);
        let rows = address_book_peer::Entity::find().all(&db.conn).await.unwrap();
        assert!(rows.iter().all(|r| r.password_enc.is_none()));
        let sealed = rows.iter().find_map(|r| r.hash_enc.clone()).unwrap();
        // No substring check against the 2-byte plaintext: random base64 contains it ~1% of
        // the time. The `v1:` sealed format is what shows it is not stored in clear.
        assert!(sealed.starts_with("v1:"), "{sealed}");
        let after = env.books.personal(alice).await.unwrap();
        assert!(after.updated_at.0 >= book.updated_at.0);
    })
    .await;
}

#[tokio::test]
async fn replacing_keeps_fields_the_pushed_peers_do_not_carry() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let book = env.books.personal(alice).await.unwrap();
        let me = Actor::User(alice);
        let stored = json!({
            "id": "111", "alias": "old", "note": "keep me", "forceAlwaysRelay": "true",
            "rdpPort": "3390", "rdpUsername": "r", "loginName": "l", "hash": "h-old",
            "tags": ["a"], "kept": { "x": 1 }, "replaced": 1
        });
        env.books
            .add_peer(me, &book.guid, peer(stored))
            .await
            .unwrap();
        env.books
            .add_peer(me, &book.guid, peer(json!({ "id": "gone" })))
            .await
            .unwrap();
        // What a legacy client pushes: its seven fields, plus a new unknown one.
        let pushed = json!({
            "id": "111", "alias": "new", "hash": "h-new", "tags": ["a", "b"],
            "username": "u", "hostname": "", "platform": "", "replaced": 2
        });

        env.books
            .replace_contents(me, &book.guid, replacement(vec![pushed], vec![]))
            .await
            .unwrap();
        let contents = env.books.contents(me, &book.guid).await.unwrap();

        assert_eq!(contents.peers.len(), 1, "the peer not pushed is gone");
        let p = &contents.peers[0];
        assert_eq!((p.alias.as_str(), p.username.as_str()), ("new", "u"));
        assert_eq!(p.tags, ["a", "b"]);
        assert_eq!(p.secret, PeerSecret::Hash("h-new".to_owned()));
        assert_eq!(p.note, "keep me");
        assert!(p.force_always_relay);
        assert_eq!(
            (p.rdp_port.as_str(), p.rdp_username.as_str()),
            ("3390", "r")
        );
        assert_eq!(p.login_name, "l");
        assert_eq!(p.extra["kept"], json!({ "x": 1 }));
        assert_eq!(p.extra["replaced"], json!(2));
    })
    .await;
}

#[tokio::test]
async fn an_empty_pushed_hash_clears_the_stored_one() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let book = env.books.personal(alice).await.unwrap();
        let me = Actor::User(alice);
        let stored = peer(json!({ "id": "1", "hash": "h" }));
        env.books.add_peer(me, &book.guid, stored).await.unwrap();

        let pushed = replacement(vec![json!({ "id": "1", "hash": "" })], vec![]);
        env.books
            .replace_contents(me, &book.guid, pushed)
            .await
            .unwrap();

        let contents = env.books.contents(me, &book.guid).await.unwrap();
        assert_eq!(contents.peers[0].secret, PeerSecret::Hash(String::new()));
    })
    .await;
}

#[tokio::test]
async fn tags_replace_the_old_set_and_colours_fall_back_to_the_stored_one_then_the_default() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let book = env.books.personal(alice).await.unwrap();
        let me = Actor::User(alice);
        for (name, color) in [("kept", 0xFF01_0203), ("dropped", 0xFF04_0506)] {
            let tag = Tag {
                name: name.to_owned(),
                color,
            };
            env.books.add_tag(me, &book.guid, tag).await.unwrap();
        }
        let tags = vec![
            tag("kept", None),
            tag("fresh", None),
            tag("explicit", Some(7)),
            tag("kept", Some(99)),
        ];

        env.books
            .replace_contents(me, &book.guid, replacement(vec![], tags))
            .await
            .unwrap();

        let stored = env.books.tags(me, &book.guid).await.unwrap();
        let stored: Vec<_> = stored.iter().map(|t| (t.name.as_str(), t.color)).collect();
        assert_eq!(
            stored,
            [
                ("explicit", 7),
                ("fresh", DEFAULT_COLOR),
                ("kept", 0xFF01_0203)
            ],
            "a repeated name keeps its first occurrence"
        );
    })
    .await;
}

#[tokio::test]
async fn repeated_peer_ids_keep_the_first_and_count_once() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let book = env.books.personal(alice).await.unwrap();
        let me = Actor::User(alice);
        common::set(&env.settings, "addressbook.max_peers", json!(2)).await;
        let pushed = replacement(
            vec![
                json!({ "id": "1", "alias": "first" }),
                json!({ "id": "1", "alias": "second" }),
                json!({ "id": "2" }),
            ],
            vec![],
        );

        env.books
            .replace_contents(me, &book.guid, pushed)
            .await
            .unwrap();

        let contents = env.books.contents(me, &book.guid).await.unwrap();
        let aliases: Vec<_> = contents.peers.iter().map(|p| p.alias.as_str()).collect();
        assert_eq!(aliases, ["first", ""]);
    })
    .await;
}

#[tokio::test]
async fn too_many_peers_are_refused_and_the_book_is_unchanged() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let book = env.books.personal(alice).await.unwrap();
        let me = Actor::User(alice);
        common::set(&env.settings, "addressbook.max_peers", json!(2)).await;
        let stored = peer(json!({ "id": "keep" }));
        env.books.add_peer(me, &book.guid, stored).await.unwrap();
        let three = replacement(
            vec![json!({"id":"1"}), json!({"id":"2"}), json!({"id":"3"})],
            vec![],
        );
        let two = replacement(vec![json!({"id":"1"}), json!({"id":"2"})], vec![]);

        let refused = env.books.replace_contents(me, &book.guid, three).await;
        let before = env.peer_ids(me, &book.guid).await;
        let accepted = env.books.replace_contents(me, &book.guid, two).await;

        assert!(
            matches!(refused, Err(AddressBookError::Full { max: 2 })),
            "{refused:?}"
        );
        assert_eq!(before, ["keep"]);
        accepted.unwrap();
        assert_eq!(env.peer_ids(me, &book.guid).await, ["1", "2"]);
    })
    .await;
}

#[tokio::test]
async fn bad_tags_are_refused_before_anything_changes() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let book = env.books.personal(alice).await.unwrap();
        let me = Actor::User(alice);
        let stored = peer(json!({ "id": "keep" }));
        env.books.add_peer(me, &book.guid, stored).await.unwrap();
        let too_many: Vec<_> = (0..1001).map(|i| tag(&format!("t{i}"), None)).collect();
        let cases = [
            vec![tag("", None)],
            vec![tag("bell\u{7}", None)],
            vec![tag(&"x".repeat(65), None)],
            vec![tag("ok", Some(-1))],
            vec![tag("ok", Some(0x1_0000_0000))],
            too_many,
        ];

        for tags in cases {
            let pushed = replacement(vec![json!({ "id": "new" })], tags);
            let result = env.books.replace_contents(me, &book.guid, pushed).await;
            assert!(
                matches!(result, Err(AddressBookError::Invalid { .. })),
                "{result:?}"
            );
        }

        assert_eq!(env.peer_ids(me, &book.guid).await, ["keep"]);
        let tag_rows = address_book_tag::Entity::find()
            .count(&db.conn)
            .await
            .unwrap();
        assert_eq!(tag_rows, 0);
    })
    .await;
}

#[tokio::test]
async fn pushing_nothing_empties_the_book() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let book = env.books.personal(alice).await.unwrap();
        let me = Actor::User(alice);
        env.books
            .add_peer(me, &book.guid, peer(json!({ "id": "1" })))
            .await
            .unwrap();
        let t = Tag {
            name: "t".to_owned(),
            color: 1,
        };
        env.books.add_tag(me, &book.guid, t).await.unwrap();

        env.books
            .replace_contents(me, &book.guid, replacement(vec![], vec![]))
            .await
            .unwrap();

        let contents = env.books.contents(me, &book.guid).await.unwrap();
        assert!(contents.peers.is_empty() && contents.tags.is_empty());
    })
    .await;
}

#[tokio::test]
async fn rules_apply_to_whole_book_operations() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let bob = env.user("bob").await;
        let personal = env.books.personal(alice).await.unwrap();
        let shared = env.shared_with("Team", bob, Rule::ReadOnly).await;
        let push = || replacement(vec![json!({ "id": "1" })], vec![]);

        let foreign_read = env.books.contents(Actor::User(bob), &personal.guid).await;
        let foreign_write = env
            .books
            .replace_contents(Actor::User(bob), &personal.guid, push())
            .await;
        let read_only_read = env.books.contents(Actor::User(bob), &shared.guid).await;
        let read_only_write = env
            .books
            .replace_contents(Actor::User(bob), &shared.guid, push())
            .await;
        let missing = env.books.contents(Actor::User(bob), "no-such-book").await;

        assert!(is_not_found(foreign_read));
        assert!(is_not_found(foreign_write));
        assert!(is_not_found(missing));
        read_only_read.unwrap();
        assert!(is_forbidden(read_only_write));
        assert!(
            env.peer_ids(Actor::User(alice), &personal.guid)
                .await
                .is_empty()
        );
    })
    .await;
}
