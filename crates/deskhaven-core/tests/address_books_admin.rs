//! AddressBookService, the console's side: the paged list of shared books with counts, shares
//! with subject names, peer search, guarded writes (audit snapshots in the write's own
//! transaction, no secrets, rollback when the guard refuses) and the batch import of devices.
//! Every test runs on SQLite and PostgreSQL.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::net::{IpAddr, Ipv4Addr};

use common::ab::{Env, bare, env, is_not_found, page, peer};
use deskhaven_core::addressbooks::{
    Actor, AddressBookError, BookQuery, BookSort, BookSortField, Rule, SharedBookUpdate, Subject,
    Tag,
};
use deskhaven_core::clock::SystemClock;
use deskhaven_core::console_audit::{
    Actor as AuditActor, ConsoleAuditLog, NewConsoleAudit, Refused, TxGuard,
};
use deskhaven_core::devices::{DeviceRepository, Registration};
use deskhaven_db::entity::{address_book_peer, console_audit, device};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, EntityTrait};
use serde_json::{Value, json};

fn query(q: Option<&str>, sort: Option<BookSort>, page_no: u64, size: u64) -> BookQuery {
    BookQuery {
        q: q.map(str::to_owned),
        sort,
        page: page(page_no, size),
    }
}

/// A guard that records the snapshots it sees under `action`.
fn recording(db: &TestDb, action: &str) -> TxGuard<Value> {
    let log = ConsoleAuditLog::new(
        db.conn.clone(),
        std::sync::Arc::new(deskhaven_core::clock::SystemClock),
    );
    let action = action.to_owned();
    TxGuard::new(&log, move |before, after| {
        Ok(NewConsoleAudit {
            actor: AuditActor::System,
            action,
            target_type: "address_book".to_owned(),
            target_id: String::new(),
            before: before.cloned(),
            after: after.cloned(),
            ip: None,
        })
    })
}

fn refusing(db: &TestDb) -> TxGuard<Value> {
    let log = ConsoleAuditLog::new(db.conn.clone(), std::sync::Arc::new(SystemClock));
    TxGuard::new(&log, |_, _| Err(Refused::new("nope")))
}

async fn audit_rows(db: &TestDb) -> Vec<console_audit::Model> {
    console_audit::Entity::find().all(&db.conn).await.unwrap()
}

async fn register(db: &TestDb, rustdesk_id: &str, hostname: &str, os: &str) -> i64 {
    let repo = DeviceRepository::new(db.conn.clone());
    let registration = Registration {
        rustdesk_id: rustdesk_id.to_owned(),
        uuid: rustdesk_id.as_bytes().to_vec(),
        pk: vec![1; 32],
        ip: IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7)),
        at: time::now(),
    };
    repo.upsert_registration(&registration).await.unwrap();
    let row = repo
        .find_by_rustdesk_id(rustdesk_id)
        .await
        .unwrap()
        .unwrap();
    let id = row.id;
    let mut changes: device::ActiveModel = row.into();
    changes.hostname = Set(Some(hostname.to_owned()));
    changes.username = Set(Some("alice".to_owned()));
    changes.os = Set(Some(os.to_owned()));
    changes.update(&db.conn).await.unwrap();
    id
}

/// The JSON of a stored snapshot column.
fn json_of(snapshot: &Option<String>) -> Value {
    serde_json::from_str(snapshot.as_deref().unwrap()).unwrap()
}

async fn stored_peers(env: &Env, guid: &str) -> Vec<String> {
    env.peer_ids(Actor::Admin, guid).await
}

#[tokio::test]
async fn the_admin_list_shows_shared_books_with_owners_and_counts() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        env.books.personal(alice).await.unwrap();
        let ops = env.shared("ops", None).await;
        let lab = env.shared("lab", Some(alice)).await;
        env.shared("zoo", None).await;
        let admin = Actor::Admin;
        env.books
            .add_peer(admin, &ops.guid, bare("p1"))
            .await
            .unwrap();
        env.books
            .add_peer(admin, &ops.guid, bare("p2"))
            .await
            .unwrap();
        env.books
            .add_tag(
                admin,
                &ops.guid,
                Tag {
                    name: "t".into(),
                    color: 1,
                },
            )
            .await
            .unwrap();
        env.books
            .set_share(
                admin,
                &lab.guid,
                Subject::User(env.user("bob").await),
                Rule::ReadOnly,
            )
            .await
            .unwrap();

        let all = env
            .books
            .admin_page(&query(None, None, 1, 10))
            .await
            .unwrap();
        let shape: Vec<_> = all
            .items
            .iter()
            .map(|s| {
                (
                    s.book.name.as_str(),
                    s.owner.as_deref(),
                    s.peer_count,
                    s.tag_count,
                    s.share_count,
                )
            })
            .collect();
        assert_eq!(all.total, 3);
        assert_eq!(
            shape,
            [
                ("lab", Some("alice"), 0, 0, 1),
                ("ops", None, 2, 1, 0),
                ("zoo", None, 0, 0, 0)
            ]
        );
        let found = env
            .books
            .admin_page(&query(Some("O"), None, 1, 10))
            .await
            .unwrap();
        assert_eq!(found.total, 2, "Ops and Zoo contain an o");
        let newest = BookSort {
            field: BookSortField::CreatedAt,
            descending: true,
        };
        let paged = env
            .books
            .admin_page(&query(None, Some(newest), 2, 2))
            .await
            .unwrap();
        assert_eq!((paged.total, paged.items.len()), (3, 1));
        assert_eq!(paged.items[0].book.name, "ops");
        assert_eq!(env.books.admin_book(&ops.guid).await.unwrap().peer_count, 2);
    })
    .await;
}

#[tokio::test]
async fn a_personal_book_is_not_a_console_book() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let personal = env.books.personal(alice).await.unwrap();

        assert!(is_not_found(env.books.admin_book(&personal.guid).await));
        assert!(is_not_found(env.books.admin_book("nope").await));
    })
    .await;
}

#[tokio::test]
async fn shares_come_back_with_the_names_of_their_subjects() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let team = env.group("team", &[alice]).await;
        let book = env.shared("Ops", None).await;
        let admin = Actor::Admin;
        env.books
            .set_share(admin, &book.guid, Subject::User(alice), Rule::ReadWrite)
            .await
            .unwrap();
        env.books
            .set_share(admin, &book.guid, Subject::Group(team), Rule::ReadOnly)
            .await
            .unwrap();

        let named = env.books.named_shares(admin, &book.guid).await.unwrap();

        let shape: Vec<_> = named
            .iter()
            .map(|n| (n.share.subject, n.share.rule, n.subject_name.as_deref()))
            .collect();
        assert_eq!(
            shape,
            [
                (Subject::Group(team), Rule::ReadOnly, Some("team")),
                (Subject::User(alice), Rule::ReadWrite, Some("alice")),
            ]
        );
    })
    .await;
}

#[tokio::test]
async fn peers_are_searched_by_id_alias_hostname_username_or_note() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let book = env.shared("Ops", None).await;
        let admin = Actor::Admin;
        let patches = [
            json!({"id": "100", "alias": "Front Desk"}),
            json!({"id": "200", "hostname": "build-box"}),
            json!({"id": "300", "username": "Carol", "note": "100% sure"}),
        ];
        for patch in patches {
            env.books
                .add_peer(admin, &book.guid, peer(patch))
                .await
                .unwrap();
        }
        let search = async |q: &str| {
            let found = env
                .books
                .peers_matching(admin, &book.guid, Some(q), page(1, 10))
                .await
                .unwrap();
            found.items.into_iter().map(|p| p.id).collect::<Vec<_>>()
        };

        assert_eq!(search("front").await, ["100"]);
        assert_eq!(search("BUILD").await, ["200"]);
        assert_eq!(search("carol").await, ["300"]);
        assert_eq!(search("100").await, ["100", "300"]);
        assert_eq!(search("%").await, ["300"], "LIKE wildcards are literal");
        assert_eq!(search("").await, ["100", "200", "300"]);
    })
    .await;
}

#[tokio::test]
async fn guarded_writes_record_snapshots_without_secrets_in_their_own_transaction() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let admin = Actor::Admin;
        let book = env
            .books
            .create_shared_guarded(
                deskhaven_core::addressbooks::NewSharedBook {
                    name: "Ops".into(),
                    note: String::new(),
                    owner_user_id: None,
                },
                recording(db, "address_book.create"),
            )
            .await
            .unwrap();
        let secret = json!({"id": "p1", "alias": "a", "password": "hunter2-hunter2"});
        env.books
            .add_peer_guarded(admin, &book.guid, peer(secret), recording(db, "peer.add"))
            .await
            .unwrap();
        let change = peer(json!({"id": "p1", "alias": "b", "password": "other-secret-pw"}));
        env.books
            .update_peer_guarded(admin, &book.guid, change, recording(db, "peer.update"))
            .await
            .unwrap();
        let rename = SharedBookUpdate {
            name: Some("Ops2".into()),
            note: None,
        };
        env.books
            .update_shared_guarded(admin, &book.guid, rename, recording(db, "book.update"))
            .await
            .unwrap();
        env.books
            .delete_peers_guarded(
                admin,
                &book.guid,
                &["p1".into(), "ghost".into()],
                recording(db, "peer.delete"),
            )
            .await
            .unwrap();
        env.books
            .delete_shared_guarded(admin, &book.guid, recording(db, "book.delete"))
            .await
            .unwrap();

        let rows = audit_rows(db).await;
        let actions: Vec<_> = rows.iter().map(|r| r.action.as_str()).collect();
        assert_eq!(
            actions,
            [
                "address_book.create",
                "peer.add",
                "peer.update",
                "book.update",
                "peer.delete",
                "book.delete"
            ]
        );
        let all = serde_json::to_string(
            &rows
                .iter()
                .map(|r| (&r.before, &r.after))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert!(
            !all.contains("hunter2") && !all.contains("other-secret"),
            "{all}"
        );
        let added = json_of(&rows[1].after);
        assert_eq!(added["peer"]["id"], "p1");
        assert_eq!(
            added["peer"]["had_password"], "<redacted>",
            "the audit log masks the flag"
        );
        let updated = (json_of(&rows[2].before), json_of(&rows[2].after));
        assert_eq!(
            (
                updated.0["peer"]["alias"].as_str(),
                updated.1["peer"]["alias"].as_str()
            ),
            (Some("a"), Some("b"))
        );
        assert_eq!(json_of(&rows[3].before)["book"]["name"], "Ops");
        assert_eq!(json_of(&rows[3].after)["book"]["name"], "Ops2");
        assert_eq!(json_of(&rows[4].before)["peer_ids"], json!(["p1"]));
        assert!(rows[5].after.is_none());
    })
    .await;
}

#[tokio::test]
async fn share_and_tag_writes_are_guarded_too_and_unchanged_writes_record_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let admin = Actor::Admin;
        let alice = env.user("alice").await;
        let book = env.shared("Ops", None).await;
        let guid = &book.guid;
        let tag = || Tag {
            name: "t1".into(),
            color: 5,
        };

        env.books
            .add_tag_guarded(admin, guid, tag(), recording(db, "tag.add"))
            .await
            .unwrap();
        env.books
            .add_tag_guarded(admin, guid, tag(), recording(db, "tag.add"))
            .await
            .unwrap();
        env.books
            .update_tag_guarded(
                admin,
                guid,
                Tag {
                    name: "t1".into(),
                    color: 6,
                },
                recording(db, "tag.update"),
            )
            .await
            .unwrap();
        env.books
            .rename_tag_guarded(admin, guid, "t1", "t2", recording(db, "tag.rename"))
            .await
            .unwrap();
        env.books
            .rename_tag_guarded(admin, guid, "t2", "t2", recording(db, "tag.rename"))
            .await
            .unwrap();
        env.books
            .delete_tags_guarded(admin, guid, &["t2".into()], recording(db, "tag.delete"))
            .await
            .unwrap();
        env.books
            .delete_tags_guarded(admin, guid, &[], recording(db, "tag.delete"))
            .await
            .unwrap();
        let before = env.books.admin_book(guid).await.unwrap().book.updated_at;
        env.books
            .delete_tags_guarded(admin, guid, &["ghost".into()], recording(db, "tag.delete"))
            .await
            .unwrap();
        let after = env.books.admin_book(guid).await.unwrap().book.updated_at;
        assert_eq!(before, after, "deleting an unknown tag changes nothing");
        env.books
            .set_share_guarded(
                admin,
                guid,
                Subject::User(alice),
                Rule::ReadOnly,
                recording(db, "share.set"),
            )
            .await
            .unwrap();
        env.books
            .set_share_guarded(
                admin,
                guid,
                Subject::User(alice),
                Rule::ReadWrite,
                recording(db, "share.set"),
            )
            .await
            .unwrap();
        env.books
            .remove_share_guarded(
                admin,
                guid,
                Subject::User(alice),
                recording(db, "share.remove"),
            )
            .await
            .unwrap();
        env.books
            .remove_share_guarded(
                admin,
                guid,
                Subject::User(alice),
                recording(db, "share.remove"),
            )
            .await
            .unwrap();
        let same = SharedBookUpdate {
            name: Some("Ops".into()),
            note: None,
        };
        env.books
            .update_shared_guarded(admin, guid, same, recording(db, "book.update"))
            .await
            .unwrap();

        let rows = audit_rows(db).await;
        let actions: Vec<_> = rows.iter().map(|r| r.action.as_str()).collect();
        assert_eq!(
            actions,
            [
                "tag.add",
                "tag.update",
                "tag.rename",
                "tag.delete",
                "share.set",
                "share.set",
                "share.remove"
            ]
        );
        let granted = json_of(&rows[5].after);
        assert_eq!(json_of(&rows[5].before)["share"]["rule"], 1);
        assert_eq!(
            (
                granted["share"]["rule"].as_i64(),
                granted["share"]["subject_id"].as_i64()
            ),
            (Some(2), Some(alice))
        );
        assert_eq!(json_of(&rows[3].before)["tags"], json!(["t2"]));
    })
    .await;
}

#[tokio::test]
async fn a_refused_rule_3_attempt_writes_no_audit_row() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.user("alice").await;
        let carol = env.user("carol").await;
        let book = env.shared_with("Ops", alice, Rule::FullControl).await;
        let guid = &book.guid;
        env.books
            .set_share(Actor::Admin, guid, Subject::User(carol), Rule::FullControl)
            .await
            .unwrap();
        let me = Actor::User(alice);

        let grant = env
            .books
            .set_share_guarded(
                me,
                guid,
                Subject::User(alice),
                Rule::FullControl,
                recording(db, "share.set"),
            )
            .await;
        let demote = env
            .books
            .set_share_guarded(
                me,
                guid,
                Subject::User(carol),
                Rule::ReadOnly,
                recording(db, "share.set"),
            )
            .await;
        let revoke = env
            .books
            .remove_share_guarded(
                me,
                guid,
                Subject::User(carol),
                recording(db, "share.remove"),
            )
            .await;
        let delete = env
            .books
            .delete_shared_guarded(me, guid, recording(db, "book.delete"))
            .await;

        for result in [grant, demote, revoke] {
            assert!(
                matches!(result, Err(AddressBookError::Forbidden)),
                "{result:?}"
            );
        }
        assert!(
            matches!(delete, Err(AddressBookError::Forbidden)),
            "{delete:?}"
        );
        assert!(audit_rows(db).await.is_empty());
        assert_eq!(env.books.shares(me, guid).await.unwrap().len(), 2);
    })
    .await;
}

#[tokio::test]
async fn dot_segments_are_not_entry_ids_or_tag_names() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let book = env.shared("Ops", None).await;
        let admin = Actor::Admin;
        let guid = &book.guid;
        env.books
            .add_tag(
                admin,
                guid,
                Tag {
                    name: "ok".into(),
                    color: 1,
                },
            )
            .await
            .unwrap();

        for name in [".", ".."] {
            let tag = Tag {
                name: name.into(),
                color: 1,
            };
            let added = env.books.add_tag(admin, guid, tag).await;
            let renamed = env.books.rename_tag(admin, guid, "ok", name).await;
            let peer = deskhaven_core::addressbooks::PeerPatch::from_json(json!({"id": name}));
            for result in [added, renamed, peer.map(drop)] {
                assert!(
                    matches!(result, Err(AddressBookError::Invalid { .. })),
                    "{name}: {result:?}"
                );
            }
        }
        assert!(
            env.books
                .add_tag(
                    admin,
                    guid,
                    Tag {
                        name: "...".into(),
                        color: 1
                    }
                )
                .await
                .is_ok()
        );
    })
    .await;
}

#[tokio::test]
async fn a_refusing_guard_rolls_the_write_back() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let admin = Actor::Admin;
        let book = env.shared("Ops", None).await;
        let guid = &book.guid;
        env.books.add_peer(admin, guid, bare("keep")).await.unwrap();

        let added = env
            .books
            .add_peer_guarded(admin, guid, bare("new"), refusing(db))
            .await;
        let deleted = env
            .books
            .delete_peers_guarded(admin, guid, &["keep".into()], refusing(db))
            .await;
        let renamed = env
            .books
            .update_shared_guarded(
                admin,
                guid,
                SharedBookUpdate {
                    name: Some("X".into()),
                    note: None,
                },
                refusing(db),
            )
            .await;
        let created = env
            .books
            .create_shared_guarded(
                deskhaven_core::addressbooks::NewSharedBook {
                    name: "Other".into(),
                    note: String::new(),
                    owner_user_id: None,
                },
                refusing(db),
            )
            .await;

        for result in [
            added.map(drop),
            deleted.map(drop),
            renamed.map(drop),
            created.map(drop),
        ] {
            assert!(
                matches!(result, Err(AddressBookError::Refused(_))),
                "{result:?}"
            );
        }
        assert_eq!(stored_peers(&env, guid).await, ["keep"]);
        assert_eq!(env.books.admin_book(guid).await.unwrap().book.name, "Ops");
        assert_eq!(
            env.books
                .admin_page(&query(None, None, 1, 10))
                .await
                .unwrap()
                .total,
            1
        );
        assert!(audit_rows(db).await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn importing_devices_adds_entries_and_leaves_existing_ones_alone() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let admin = Actor::Admin;
        let book = env.shared("Ops", None).await;
        let win = register(db, "100", "DESKTOP-1", "windows / Windows 10 Pro").await;
        let lin = register(db, "200", "build-box", "linux / Ubuntu 22.04").await;
        let listed = register(db, "300", "other", "linux / Debian").await;
        env.books
            .add_peer(
                admin,
                &book.guid,
                peer(json!({"id": "300", "alias": "mine"})),
            )
            .await
            .unwrap();

        let report = env
            .books
            .import_devices_guarded(
                admin,
                &book.guid,
                &[win, lin, listed, 9999, win],
                recording(db, "book.import"),
            )
            .await
            .unwrap();

        assert_eq!(report.added, ["100", "200"]);
        assert_eq!(report.existing, ["300"]);
        assert_eq!(report.missing, [9999]);
        let page = env
            .books
            .peers(admin, &book.guid, page(1, 10))
            .await
            .unwrap();
        let shape: Vec<_> = page
            .items
            .iter()
            .map(|p| {
                (
                    p.id.as_str(),
                    p.alias.as_str(),
                    p.hostname.as_str(),
                    p.username.as_str(),
                    p.platform.as_str(),
                )
            })
            .collect();
        assert_eq!(
            shape,
            [
                ("100", "", "DESKTOP-1", "alice", "Windows"),
                ("200", "", "build-box", "alice", "Linux"),
                ("300", "mine", "", "", ""),
            ]
        );
        let rows = audit_rows(db).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(json_of(&rows[0].after)["peer_ids"], json!(["100", "200"]));
        let again = env
            .books
            .import_devices(admin, &book.guid, &[win, lin])
            .await
            .unwrap();
        assert!(again.added.is_empty());
        assert_eq!(
            audit_rows(db).await.len(),
            1,
            "nothing added, nothing recorded"
        );
    })
    .await;
}

#[tokio::test]
async fn an_import_that_adds_nothing_reports_even_over_a_lowered_limit() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let admin = Actor::Admin;
        let book = env.shared("Ops", None).await;
        let ids = [
            register(db, "100", "a", "linux").await,
            register(db, "200", "b", "linux").await,
        ];
        env.books
            .import_devices(admin, &book.guid, &ids)
            .await
            .unwrap();
        common::set(&env.settings, "addressbook.max_peers", json!(1)).await;

        let report = env
            .books
            .import_devices(admin, &book.guid, &ids)
            .await
            .unwrap();

        assert!(report.added.is_empty());
        assert_eq!(report.existing, ["100", "200"]);
    })
    .await;
}

#[tokio::test]
async fn a_device_whose_registry_values_make_no_entry_is_named_in_the_error() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let book = env.shared("Ops", None).await;
        let good = register(db, "100", "fine", "linux").await;
        let bad = register(db, "200", "bell\u{7}host", "linux").await;

        let refused = env
            .books
            .import_devices(Actor::Admin, &book.guid, &[good, bad])
            .await;

        match refused {
            Err(AddressBookError::Invalid { field, reason }) => {
                assert_eq!(field, "device_ids");
                assert!(reason.contains(&format!("device {bad}")), "{reason}");
            }
            other => panic!("{other:?}"),
        }
        assert!(
            stored_peers(&env, &book.guid).await.is_empty(),
            "all or nothing"
        );
    })
    .await;
}

#[tokio::test]
async fn an_import_that_does_not_fit_adds_nothing_and_needs_rule_2() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let admin = Actor::Admin;
        common::set(&env.settings, "addressbook.max_peers", json!(2)).await;
        let alice = env.user("alice").await;
        let book = env.shared_with("Ops", alice, Rule::ReadOnly).await;
        let ids = [
            register(db, "100", "a", "linux").await,
            register(db, "200", "b", "linux").await,
            register(db, "300", "c", "linux").await,
        ];

        let denied = env
            .books
            .import_devices(Actor::User(alice), &book.guid, &ids[..1])
            .await;
        let full = env.books.import_devices(admin, &book.guid, &ids).await;
        let too_many = env
            .books
            .import_devices(admin, &book.guid, &vec![1; 501])
            .await;

        assert!(
            matches!(denied, Err(AddressBookError::Forbidden)),
            "{denied:?}"
        );
        assert!(
            matches!(full, Err(AddressBookError::Full { max: 2 })),
            "{full:?}"
        );
        assert!(
            matches!(too_many, Err(AddressBookError::Invalid { .. })),
            "{too_many:?}"
        );
        assert!(stored_peers(&env, &book.guid).await.is_empty());
        let rows = address_book_peer::Entity::find()
            .all(&db.conn)
            .await
            .unwrap();
        assert!(rows.is_empty());
    })
    .await;
}
