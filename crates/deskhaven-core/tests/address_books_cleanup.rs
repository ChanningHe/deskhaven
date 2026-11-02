//! Deleting a group or a user removes the address book shares naming it in the same
//! transaction (shares have no foreign key to their subject); a deleted user's personal book
//! goes, its shared books stay unowned. Every test runs on SQLite and PostgreSQL.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::ab::{bare, env, is_not_found};
use deskhaven_core::addressbooks::{Actor, Rule, Share, Subject};
use deskhaven_db::entity::address_book_share::{self, SubjectType};
use deskhaven_db::entity::{address_book, address_book_peer};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

async fn share_rows(db: &TestDb, kind: SubjectType, id: i64) -> u64 {
    address_book_share::Entity::find()
        .filter(address_book_share::Column::SubjectType.eq(kind))
        .filter(address_book_share::Column::SubjectId.eq(id))
        .count(&db.conn)
        .await
        .unwrap()
}

#[tokio::test]
async fn deleting_a_group_removes_its_shares_and_nothing_else() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let doomed = env.group("doomed", &[alice]).await;
        let kept = env.group("kept", &[alice]).await;
        let book = env.shared("team", None).await;
        let guid = &book.guid;
        for (subject, rule) in [
            (Subject::Group(doomed), Rule::FullControl),
            (Subject::Group(kept), Rule::ReadOnly),
            (Subject::User(alice), Rule::ReadOnly),
        ] {
            env.books
                .set_share(Actor::Admin, guid, subject, rule)
                .await
                .unwrap();
        }

        env.groups.delete(doomed).await.unwrap();

        assert_eq!(share_rows(db, SubjectType::Group, doomed).await, 0);
        let shares = env.books.shares(Actor::Admin, guid).await.unwrap();
        assert_eq!(
            shares,
            [
                Share {
                    subject: Subject::Group(kept),
                    rule: Rule::ReadOnly
                },
                Share {
                    subject: Subject::User(alice),
                    rule: Rule::ReadOnly
                },
            ]
        );
        // The deleted group's full control no longer counts.
        let profiles = env
            .books
            .shared_profiles(alice, common::ab::page(1, 10))
            .await
            .unwrap();
        assert_eq!(profiles.items[0].rule, Rule::ReadOnly);
    })
    .await;
}

#[tokio::test]
async fn deleting_a_user_removes_its_shares_and_personal_book() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let bob = env.user("bob").await;
        let personal = env.books.personal(alice).await.unwrap();
        env.books
            .add_peer(Actor::User(alice), &personal.guid, bare("1"))
            .await
            .unwrap();
        let owned = env.shared("alice's", Some(alice)).await;
        env.books
            .add_peer(Actor::User(alice), &owned.guid, bare("2"))
            .await
            .unwrap();
        let other = env.shared_with("ops", alice, Rule::ReadWrite).await;
        env.books
            .set_share(
                Actor::Admin,
                &other.guid,
                Subject::User(bob),
                Rule::ReadOnly,
            )
            .await
            .unwrap();

        env.users.delete(alice).await.unwrap();

        assert_eq!(share_rows(db, SubjectType::User, alice).await, 0);
        assert_eq!(share_rows(db, SubjectType::User, bob).await, 1);
        let personal_row = address_book::Entity::find_by_id(personal.id)
            .one(&db.conn)
            .await;
        assert_eq!(personal_row.unwrap(), None);
        let unowned = address_book::Entity::find_by_id(owned.id)
            .one(&db.conn)
            .await;
        assert_eq!(unowned.unwrap().unwrap().owner_user_id, None);
        let peers = address_book_peer::Entity::find()
            .count(&db.conn)
            .await
            .unwrap();
        assert_eq!(peers, 1, "only the shared book's peer remains");
        // The unowned book is admin-managed now: reachable through the console only.
        assert_eq!(env.peer_ids(Actor::Admin, &owned.guid).await, ["2"]);
        assert!(is_not_found(
            env.books.tags(Actor::User(bob), &owned.guid).await
        ));
    })
    .await;
}
