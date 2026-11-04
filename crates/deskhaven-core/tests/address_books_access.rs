//! AddressBookService, authorization: the rule matrix (1 read, 2 write peers and tags, 3
//! manage shares and the book), group shares and the effective rule, owners, admin-managed
//! books, and that books out of reach look exactly like books that do not exist. Every test
//! runs on SQLite and PostgreSQL.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::ab::{Env, bare, env, is_forbidden, is_not_found, page, peer};
use deskhaven_core::addressbooks::{
    Actor, AddressBookError, AddressBookService, Rule, SharedBookUpdate, Subject, Tag,
};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::json;

fn tag(name: &str) -> Tag {
    Tag {
        name: name.to_owned(),
        color: 0xFF00_FF00,
    }
}

/// Every peer and tag write, as `actor`, each result reduced to "allowed" / the error.
async fn writes(books: &AddressBookService, actor: Actor, guid: &str) -> Vec<Result<(), String>> {
    let show = |r: Result<(), AddressBookError>| r.map_err(|e| format!("{e:?}"));
    vec![
        show(books.add_peer(actor, guid, bare("new")).await),
        show(
            books
                .update_peer(actor, guid, peer(json!({"id": "p1", "alias": "x"})))
                .await,
        ),
        show(books.add_tag(actor, guid, tag("t2")).await),
        show(books.update_tag(actor, guid, tag("t1")).await),
        show(books.rename_tag(actor, guid, "t1", "t3").await),
        show(books.delete_tags(actor, guid, &["t2".into()]).await),
        show(books.delete_peers(actor, guid, &["new".into()]).await),
    ]
}

/// Every management operation (rule 3), as `actor`.
async fn management(env: &Env, actor: Actor, guid: &str, other: i64) -> Vec<Result<(), String>> {
    let show = |r: Result<(), AddressBookError>| r.map_err(|e| format!("{e:?}"));
    let books = &env.books;
    let rename = SharedBookUpdate {
        note: Some("managed".into()),
        ..SharedBookUpdate::default()
    };
    vec![
        show(books.shares(actor, guid).await.map(drop)),
        show(
            books
                .set_share(actor, guid, Subject::User(other), Rule::ReadOnly)
                .await,
        ),
        show(books.remove_share(actor, guid, Subject::User(other)).await),
        show(books.update_shared(actor, guid, rename).await.map(drop)),
    ]
}

/// A book shared with `user` at `rule`, holding peer `p1` and tag `t1`.
async fn stocked(env: &Env, name: &str, user: i64, rule: Rule) -> String {
    let book = env.shared_with(name, user, rule).await;
    env.books
        .add_peer(Actor::Admin, &book.guid, bare("p1"))
        .await
        .unwrap();
    env.books
        .add_tag(Actor::Admin, &book.guid, tag("t1"))
        .await
        .unwrap();
    book.guid
}

#[tokio::test]
async fn rule_1_reads_but_every_write_and_management_is_forbidden() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let bob = env.user("bob").await;
        let guid = stocked(&env, "read", alice, Rule::ReadOnly).await;
        let me = Actor::User(alice);

        assert_eq!(env.peer_ids(me, &guid).await, ["p1"]);
        assert_eq!(env.books.tags(me, &guid).await.unwrap(), [tag("t1")]);
        for result in writes(&env.books, me, &guid).await {
            assert_eq!(result, Err("Forbidden".to_owned()));
        }
        for result in management(&env, me, &guid, bob).await {
            assert_eq!(result, Err("Forbidden".to_owned()));
        }
        assert_eq!(env.peer_ids(me, &guid).await, ["p1"]);
    })
    .await;
}

#[tokio::test]
async fn rule_2_writes_peers_and_tags_but_cannot_manage() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let bob = env.user("bob").await;
        let guid = stocked(&env, "write", alice, Rule::ReadWrite).await;
        let me = Actor::User(alice);

        for result in writes(&env.books, me, &guid).await {
            assert_eq!(result, Ok(()));
        }
        for result in management(&env, me, &guid, bob).await {
            assert_eq!(result, Err("Forbidden".to_owned()));
        }
        assert!(is_forbidden(env.books.delete_shared(me, &guid).await));
        let tags = env.books.tags(me, &guid).await.unwrap();
        assert_eq!(tags, [tag("t3")]);
    })
    .await;
}

#[tokio::test]
async fn rule_3_sharee_writes_and_manages_shares_up_to_rule_2() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let bob = env.user("bob").await;
        let guid = stocked(&env, "full", alice, Rule::FullControl).await;
        let me = Actor::User(alice);

        for result in writes(&env.books, me, &guid).await {
            assert_eq!(result, Ok(()));
        }
        for result in management(&env, me, &guid, bob).await {
            assert_eq!(result, Ok(()));
        }
        env.books
            .set_share(me, &guid, Subject::User(bob), Rule::ReadWrite)
            .await
            .unwrap();
        assert_eq!(env.peer_ids(Actor::User(bob), &guid).await, ["p1"]);
    })
    .await;
}

#[tokio::test]
async fn rule_3_sharee_can_neither_grant_rule_3_nor_delete_the_book() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let bob = env.user("bob").await;
        let guid = stocked(&env, "full", alice, Rule::FullControl).await;
        let me = Actor::User(alice);

        let grant = env
            .books
            .set_share(me, &guid, Subject::User(bob), Rule::FullControl)
            .await;
        assert!(is_forbidden(grant));
        assert!(is_forbidden(env.books.delete_shared(me, &guid).await));
        assert!(is_not_found(env.books.tags(Actor::User(bob), &guid).await));
        assert_eq!(env.books.shares(me, &guid).await.unwrap().len(), 1);
        env.books.delete_shared(Actor::Admin, &guid).await.unwrap();
        assert!(is_not_found(env.books.tags(me, &guid).await));
    })
    .await;
}

#[tokio::test]
async fn rule_3_sharee_cannot_change_or_revoke_another_rule_3_grant() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let carol = env.user("carol").await;
        let managers = env.group("managers", &[carol]).await;
        let guid = stocked(&env, "full", alice, Rule::FullControl).await;
        let admin = Actor::Admin;
        env.books
            .set_share(admin, &guid, Subject::User(carol), Rule::FullControl)
            .await
            .unwrap();
        env.books
            .set_share(admin, &guid, Subject::Group(managers), Rule::FullControl)
            .await
            .unwrap();
        let me = Actor::User(alice);

        for subject in [Subject::User(carol), Subject::Group(managers)] {
            let demote = env
                .books
                .set_share(me, &guid, subject, Rule::ReadOnly)
                .await;
            assert!(is_forbidden(demote), "{subject:?}");
            let revoke = env.books.remove_share(me, &guid, subject).await;
            assert!(is_forbidden(revoke), "{subject:?}");
        }
        assert_eq!(env.books.shares(me, &guid).await.unwrap().len(), 3);
        // The console may.
        env.books
            .set_share(admin, &guid, Subject::User(carol), Rule::ReadOnly)
            .await
            .unwrap();
        env.books
            .remove_share(admin, &guid, Subject::Group(managers))
            .await
            .unwrap();
    })
    .await;
}

#[tokio::test]
async fn books_out_of_reach_are_indistinguishable_from_missing_ones() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let bob = env.user("bob").await;
        let guid = stocked(&env, "theirs", bob, Rule::FullControl).await;
        let me = Actor::User(alice);

        for target in [
            guid.as_str(),
            "00000000-0000-7000-8000-000000000000",
            &"x".repeat(500),
        ] {
            assert!(is_not_found(env.books.peers(me, target, page(1, 10)).await));
            assert!(is_not_found(env.books.tags(me, target).await));
            for result in writes(&env.books, me, target).await {
                assert_eq!(result, Err("NotFound".to_owned()), "{target}");
            }
            for result in management(&env, me, target, bob).await {
                assert_eq!(result, Err("NotFound".to_owned()), "{target}");
            }
        }
        // A share to a missing subject is reported only to someone with full control.
        let ghost = Subject::User(999_999);
        let denied = env.books.set_share(me, &guid, ghost, Rule::ReadOnly).await;
        let refused = env
            .books
            .set_share(Actor::Admin, &guid, ghost, Rule::ReadOnly)
            .await;
        assert!(is_not_found(denied));
        assert!(
            matches!(refused, Err(AddressBookError::SubjectNotFound)),
            "{refused:?}"
        );
    })
    .await;
}

#[tokio::test]
async fn group_shares_grant_access_and_the_highest_rule_wins() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let carol = env.user("carol").await;
        let readers = env.group("readers", &[alice, carol]).await;
        let writers = env.group("writers", &[alice]).await;
        let book = env.shared("team", None).await;
        let admin = Actor::Admin;
        let guid = &book.guid;
        env.books
            .set_share(admin, guid, Subject::Group(readers), Rule::ReadOnly)
            .await
            .unwrap();
        env.books.add_peer(admin, guid, bare("p1")).await.unwrap();

        assert!(is_forbidden(
            env.books
                .add_peer(Actor::User(alice), guid, bare("a"))
                .await
        ));
        env.books
            .set_share(admin, guid, Subject::Group(writers), Rule::ReadWrite)
            .await
            .unwrap();
        env.books
            .add_peer(Actor::User(alice), guid, bare("a"))
            .await
            .unwrap();
        assert!(is_forbidden(
            env.books
                .add_peer(Actor::User(carol), guid, bare("c"))
                .await
        ));
        // A direct read-only share does not lower what a group grants.
        env.books
            .set_share(admin, guid, Subject::User(alice), Rule::ReadOnly)
            .await
            .unwrap();
        env.books
            .add_peer(Actor::User(alice), guid, bare("b"))
            .await
            .unwrap();
        // Leaving the group takes its rule away.
        env.groups.remove_member(writers, alice).await.unwrap();
        assert!(is_forbidden(
            env.books
                .add_peer(Actor::User(alice), guid, bare("d"))
                .await
        ));
        env.groups.remove_member(readers, carol).await.unwrap();
        assert!(is_not_found(env.books.tags(Actor::User(carol), guid).await));
    })
    .await;
}

#[tokio::test]
async fn owners_have_full_control_and_unowned_books_need_shares() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let bob = env.user("bob").await;
        let book = env.shared("alice's", Some(alice)).await;
        let admin_managed = env.shared("ops", None).await;

        env.books
            .set_share(
                Actor::User(alice),
                &book.guid,
                Subject::User(bob),
                Rule::ReadOnly,
            )
            .await
            .unwrap();
        env.books
            .add_peer(Actor::User(alice), &book.guid, bare("1"))
            .await
            .unwrap();
        assert_eq!(env.peer_ids(Actor::User(bob), &book.guid).await, ["1"]);
        // Nobody reaches an unowned book without a share; the console does.
        assert!(is_not_found(
            env.books
                .tags(Actor::User(alice), &admin_managed.guid)
                .await
        ));
        env.books
            .add_peer(Actor::Admin, &admin_managed.guid, bare("2"))
            .await
            .unwrap();
    })
    .await;
}

#[tokio::test]
async fn shared_profiles_list_reachable_books_with_rules_and_owners() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let bob = env.user("bob").await;
        let team = env.group("team", &[alice]).await;
        env.books.personal(alice).await.unwrap();
        env.shared("b-owned", Some(alice)).await;
        let direct = env.shared_with("c-direct", alice, Rule::ReadOnly).await;
        let grouped = env.shared("a-group", Some(bob)).await;
        env.books
            .set_share(
                Actor::Admin,
                &grouped.guid,
                Subject::Group(team),
                Rule::ReadWrite,
            )
            .await
            .unwrap();
        env.books
            .set_share(
                Actor::Admin,
                &direct.guid,
                Subject::Group(team),
                Rule::FullControl,
            )
            .await
            .unwrap();
        env.shared("d-unrelated", Some(bob)).await;

        let first = env.books.shared_profiles(alice, page(1, 2)).await.unwrap();
        let second = env.books.shared_profiles(alice, page(2, 2)).await.unwrap();

        assert_eq!(first.total, 3);
        let summary: Vec<_> = first
            .items
            .iter()
            .chain(&second.items)
            .map(|p| (p.book.name.as_str(), p.owner.as_deref(), p.rule))
            .collect();
        assert_eq!(
            summary,
            [
                ("a-group", Some("bob"), Rule::ReadWrite),
                ("b-owned", Some("alice"), Rule::FullControl),
                ("c-direct", None, Rule::FullControl),
            ]
        );
        let none = env
            .books
            .shared_profiles(env.user("carol").await, page(1, 10))
            .await;
        assert_eq!(none.unwrap().total, 0);
    })
    .await;
}
