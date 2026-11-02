//! AddressBookService under concurrency: the capacity check holds against concurrent adds,
//! a revocation and a write to the same book serialize, and deleting a user while shares
//! change neither deadlocks nor fails (lock order subject -> book -> share rows). Every
//! test runs on SQLite and PostgreSQL (the races matter on PostgreSQL, whose transactions
//! interleave; SQLite serializes writers).

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::ab::{bare, env};
use deskhaven_core::addressbooks::{Actor, AddressBookError, Rule, Subject};
use deskhaven_db::entity::address_book_peer;
use deskhaven_db::entity::address_book_share::{self, SubjectType};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
use serde_json::json;

const MAX_PEERS: u32 = 5;
const ADDERS: usize = 12;
const ROUNDS: usize = 8;
/// Without the book lock in `forget_user` the deadlock showed up in two runs of three.
const DELETE_ROUNDS: usize = 25;

#[tokio::test]
async fn concurrent_adds_never_exceed_max_peers() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let book = env.books.personal(alice).await.unwrap();
        common::set(&env.settings, "addressbook.max_peers", json!(MAX_PEERS)).await;

        let tasks: Vec<_> = (0..ADDERS)
            .map(|i| {
                let (books, guid) = (env.books.clone(), book.guid.clone());
                tokio::spawn(async move {
                    books
                        .add_peer(Actor::User(alice), &guid, bare(&format!("p{i}")))
                        .await
                })
            })
            .collect();
        let mut added = 0;
        for task in tasks {
            match task.await.unwrap() {
                Ok(()) => added += 1,
                Err(AddressBookError::Full { max: MAX_PEERS }) => {}
                Err(other) => panic!("{}: unexpected {other:?}", db.backend),
            }
        }

        assert_eq!(added, MAX_PEERS);
        let stored = address_book_peer::Entity::find()
            .count(&db.conn)
            .await
            .unwrap();
        assert_eq!(stored, u64::from(MAX_PEERS));
    })
    .await;
}

#[tokio::test]
async fn a_revocation_racing_a_write_never_lets_the_write_land_after_it() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        for round in 0..ROUNDS {
            let book = env
                .shared_with(&format!("team {round}"), alice, Rule::ReadWrite)
                .await;
            let id = format!("r{round}");

            let (books, guid, peer_id) = (env.books.clone(), book.guid.clone(), id.clone());
            let add = tokio::spawn(async move {
                books
                    .add_peer(Actor::User(alice), &guid, bare(&peer_id))
                    .await
            });
            let (books, guid) = (env.books.clone(), book.guid.clone());
            let revoke = tokio::spawn(async move {
                books
                    .remove_share(Actor::Admin, &guid, Subject::User(alice))
                    .await
            });
            let added = add.await.unwrap();
            revoke.await.unwrap().unwrap();

            let stored = address_book_peer::Entity::find_by_id((book.id, id))
                .one(&db.conn)
                .await
                .unwrap();
            match added {
                // Committed before the revocation (both lock the book row).
                Ok(()) => assert!(stored.is_some(), "{}", db.backend),
                // Ran after it: the book is out of reach now.
                Err(AddressBookError::NotFound) => assert!(stored.is_none()),
                Err(other) => panic!("{}: unexpected {other:?}", db.backend),
            }
            let late = env
                .books
                .add_peer(Actor::User(alice), &book.guid, bare("late"))
                .await;
            assert!(matches!(late, Err(AddressBookError::NotFound)), "{late:?}");
        }
    })
    .await;
}

#[tokio::test]
async fn deleting_a_user_while_shares_change_neither_deadlocks_nor_fails() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let xavier = env.user("xavier").await;
        let theirs = env.shared("xavier's", Some(xavier)).await;
        for round in 0..DELETE_ROUNDS {
            let doomed = env.user(&format!("yann{round}")).await;
            let owned = env.shared(&format!("yann's {round}"), Some(doomed)).await;
            env.books.personal(doomed).await.unwrap();
            let admin = Actor::Admin;
            env.books
                .set_share(admin, &owned.guid, Subject::User(xavier), Rule::ReadOnly)
                .await
                .unwrap();
            env.books
                .set_share(admin, &theirs.guid, Subject::User(doomed), Rule::ReadOnly)
                .await
                .unwrap();
            // A share naming the owner of its own book: refused by the service now, but rows
            // written before (or by hand) must not deadlock the owner's deletion either.
            let owner_share = address_book_share::ActiveModel {
                address_book_id: Set(owned.id),
                subject_type: Set(SubjectType::User),
                subject_id: Set(doomed),
                rule: Set(Rule::ReadOnly),
            };
            owner_share.insert(&db.conn).await.unwrap();
            let doomed_too = env
                .shared(&format!("yann's other {round}"), Some(doomed))
                .await;

            let users = env.users.clone();
            let delete = tokio::spawn(async move { users.delete(doomed).await });
            let ops: Vec<_> = (0..6)
                .map(|op| {
                    let books = env.books.clone();
                    let (owned, theirs) = (owned.guid.clone(), theirs.guid.clone());
                    let other = doomed_too.guid.clone();
                    tokio::spawn(async move {
                        match op {
                            0 => {
                                books
                                    .set_share(
                                        admin,
                                        &theirs,
                                        Subject::User(doomed),
                                        Rule::ReadWrite,
                                    )
                                    .await
                            }
                            1 => {
                                books
                                    .remove_share(admin, &theirs, Subject::User(doomed))
                                    .await
                            }
                            2 => {
                                books
                                    .set_share(
                                        admin,
                                        &owned,
                                        Subject::User(xavier),
                                        Rule::FullControl,
                                    )
                                    .await
                            }
                            3 => {
                                books
                                    .remove_share(admin, &owned, Subject::User(xavier))
                                    .await
                            }
                            4 => {
                                books
                                    .remove_share(admin, &owned, Subject::User(doomed))
                                    .await
                            }
                            _ => books.delete_shared(admin, &other).await,
                        }
                    })
                })
                .collect();
            delete.await.unwrap().unwrap();
            for op in ops {
                match op.await.unwrap() {
                    Ok(()) | Err(AddressBookError::SubjectNotFound) => {}
                    Err(other) => panic!("{}: round {round}: {other:?}", db.backend),
                }
            }

            let left = address_book_share::Entity::find()
                .filter(address_book_share::Column::SubjectType.eq(SubjectType::User))
                .filter(address_book_share::Column::SubjectId.eq(doomed))
                .count(&db.conn)
                .await
                .unwrap();
            assert_eq!(left, 0, "{}: round {round}", db.backend);
        }
    })
    .await;
}

#[tokio::test]
async fn sharing_a_book_with_its_owner_is_refused() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let book = env.shared("alice's", Some(alice)).await;

        let result = env
            .books
            .set_share(
                Actor::Admin,
                &book.guid,
                Subject::User(alice),
                Rule::ReadOnly,
            )
            .await;

        let invalid = matches!(
            result,
            Err(AddressBookError::Invalid {
                field: "subject",
                ..
            })
        );
        assert!(invalid, "{result:?}");
        let shares = env.books.shares(Actor::Admin, &book.guid).await.unwrap();
        assert!(shares.is_empty());
    })
    .await;
}
