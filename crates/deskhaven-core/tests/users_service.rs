//! UserService: creation and normalization, uniqueness, local authentication, rehash on
//! login, last-owner protection, events. Every test runs on SQLite and PostgreSQL.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;

use deskhaven_config::settings::{Change, PinSet};
use deskhaven_core::crypto::{MasterKey, SecretBox};
use deskhaven_core::groups::{GroupService, NewGroup};
use deskhaven_core::pagination::PageRequest;
use deskhaven_core::settings::{SettingsService, UpdateRequest};
use deskhaven_core::users::{
    AuthError, AuthSource, Credential, HashParams, NewUser, Password, PasswordHasher,
    PolicyViolation, ProfileUpdate, Role, Status, User, UserError, UserEvent, UserQuery,
    UserService,
};
use deskhaven_db::entity::{address_book, address_book_peer, user};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, EntityTrait};
use serde_json::json;
use tokio::sync::broadcast::error::TryRecvError;

const PASSWORD: &str = "tundra-Lamp-42";
const FAST: HashParams = HashParams {
    memory_kib: 1024,
    iterations: 1,
    parallelism: 1,
};
const DEFAULT_PREFIX: &str = "$argon2id$v=19$m=19456,t=2,p=1$";

async fn settings(db: &TestDb) -> SettingsService {
    let crypto = Arc::new(SecretBox::new(&MasterKey::from_bytes(&[7; 32])));
    let (settings, _) = SettingsService::load(db.conn.clone(), crypto, PinSet::empty())
        .await
        .unwrap();
    settings
}

async fn service_with(db: &TestDb, params: HashParams) -> UserService {
    let hasher = PasswordHasher::new(params).unwrap();
    UserService::new(db.conn.clone(), settings(db).await, hasher)
}

async fn service(db: &TestDb) -> UserService {
    service_with(db, FAST).await
}

fn pw(s: &str) -> Password {
    Password::new(s)
}

fn local(username: &str) -> NewUser {
    NewUser::new(
        username,
        Credential::Local {
            password: pw(PASSWORD),
        },
    )
}

fn with_role(new: NewUser, role: Role) -> NewUser {
    NewUser { role, ..new }
}

async fn row(db: &TestDb, id: i64) -> user::Model {
    user::Entity::find_by_id(id)
        .one(&db.conn)
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn create_normalizes_username_and_email_and_hashes_the_password() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;
        let new = NewUser {
            email: Some("Alice.Smith@Example.COM".to_owned()),
            ..local("Alice.Smith")
        };

        let created = svc.create(new).await.unwrap();

        assert_eq!(created.username, "Alice.Smith");
        assert_eq!(created.display_name, "Alice.Smith");
        assert_eq!(created.email.as_deref(), Some("Alice.Smith@Example.COM"));
        assert_eq!(created.auth_source, AuthSource::Local);
        assert_eq!(created.role, Role::User);
        assert_eq!(created.status, Status::Active);
        let stored = row(db, created.id).await;
        assert_eq!(stored.username_norm, "alice.smith");
        assert_eq!(
            stored.email_norm.as_deref(),
            Some("alice.smith@example.com")
        );
        assert!(
            stored
                .password_hash
                .as_deref()
                .unwrap()
                .starts_with("$argon2id$")
        );
        let found = svc.get_by_username("ALICE.SMITH").await.unwrap();
        assert_eq!(found, created);
        assert_eq!(svc.get(created.id).await.unwrap(), created);
    })
    .await;
}

#[tokio::test]
async fn username_and_email_are_unique_case_insensitively() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;
        let first = NewUser {
            email: Some("bob@example.com".to_owned()),
            ..local("bob")
        };
        svc.create(first).await.unwrap();

        let same_name = svc.create(local("BOB")).await;
        let same_email = NewUser {
            email: Some("BOB@Example.com".to_owned()),
            ..local("robert")
        };
        let same_email = svc.create(same_email).await;

        assert!(
            matches!(same_name, Err(UserError::Conflict { field: "username" })),
            "{same_name:?}"
        );
        assert!(
            matches!(same_email, Err(UserError::Conflict { field: "email" })),
            "{same_email:?}"
        );
    })
    .await;
}

#[tokio::test]
async fn create_rejects_invalid_usernames_and_emails() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;

        for bad in ["", "has space", "名字", "semi;colon", &"a".repeat(65)] {
            let result = svc.create(local(bad)).await;
            assert!(
                matches!(
                    result,
                    Err(UserError::Invalid {
                        field: "username",
                        ..
                    })
                ),
                "{bad:?}: {result:?}"
            );
        }
        for bad in [
            "",
            "no-at-sign",
            "@example.com",
            "a@",
            "a@b@c",
            "a b@example.com",
        ] {
            let new = NewUser {
                email: Some(bad.to_owned()),
                ..local("carol")
            };
            let result = svc.create(new).await;
            assert!(
                matches!(result, Err(UserError::Invalid { field: "email", .. })),
                "{bad:?}: {result:?}"
            );
        }
        let longest = "a".repeat(64);
        assert!(svc.create(local(&longest)).await.is_ok());
        assert!(svc.create(local("x.y_z-1@corp")).await.is_ok());
    })
    .await;
}

#[tokio::test]
async fn create_enforces_the_password_policy_from_settings() {
    each_migrated_backend(async |db: &TestDb| {
        let settings = settings(db).await;
        let svc = UserService::new(
            db.conn.clone(),
            settings.clone(),
            PasswordHasher::new(FAST).unwrap(),
        );
        let with_password = |name: &str, password: &str| {
            NewUser::new(
                name,
                Credential::Local {
                    password: pw(password),
                },
            )
        };

        let short = svc.create(with_password("dave", "Short-9")).await;
        let common = svc.create(with_password("dave", "Password123")).await;
        assert!(
            matches!(
                short,
                Err(UserError::PolicyViolation {
                    reason: PolicyViolation::TooShort { min: 10 }
                })
            ),
            "{short:?}"
        );
        assert!(
            matches!(
                common,
                Err(UserError::PolicyViolation {
                    reason: PolicyViolation::Common
                })
            ),
            "{common:?}"
        );

        let raise = Change {
            key: "auth.password.min_length".to_owned(),
            value: json!(16),
            version: 0,
        };
        let request = UpdateRequest {
            changes: vec![raise],
            actor: None,
            reason: None,
        };
        settings.update(request).await.unwrap();
        let fifteen = svc.create(with_password("dave", "tundra-Lamp-42x")).await;
        assert!(
            matches!(
                fifteen,
                Err(UserError::PolicyViolation {
                    reason: PolicyViolation::TooShort { min: 16 }
                })
            ),
            "{fifteen:?}"
        );
    })
    .await;
}

#[tokio::test]
async fn external_users_have_no_password_and_cannot_log_in_locally() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;

        let ldap = svc
            .create(NewUser::new("erin", Credential::Ldap))
            .await
            .unwrap();
        let oidc = svc
            .create(NewUser::new("frank", Credential::Oidc))
            .await
            .unwrap();

        assert_eq!(ldap.auth_source, AuthSource::Ldap);
        assert_eq!(oidc.auth_source, AuthSource::Oidc);
        assert_eq!(row(db, ldap.id).await.password_hash, None);
        let login = svc.authenticate_local("erin", &pw(PASSWORD)).await;
        assert!(matches!(login, Err(AuthError::InvalidCredentials)));
        let set = svc.set_password(ldap.id, &pw(PASSWORD), false).await;
        assert!(matches!(set, Err(UserError::NotLocal)), "{set:?}");
    })
    .await;
}

#[tokio::test]
async fn authentication_failures_are_indistinguishable() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;
        let grace = svc.create(local("Grace")).await.unwrap();
        svc.create(NewUser::new("heidi", Credential::Ldap))
            .await
            .unwrap();

        let ok = svc
            .authenticate_local("grace", &pw(PASSWORD))
            .await
            .unwrap();
        assert_eq!(ok.user.id, grace.id);

        let too_long = "x".repeat(10_000);
        let failures = [
            ("grace", "wrong-password-1"),
            ("nobody", PASSWORD),
            ("heidi", PASSWORD),
            ("not a valid name", PASSWORD),
            ("grace", too_long.as_str()),
        ];
        for (username, password) in failures {
            let result = svc.authenticate_local(username, &pw(password)).await;
            assert!(
                matches!(result, Err(AuthError::InvalidCredentials)),
                "{username}: {result:?}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn disabled_users_cannot_authenticate() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;
        let ivan = svc.create(local("ivan")).await.unwrap();

        let disabled = svc.set_status(ivan.id, Status::Disabled).await.unwrap();
        assert_eq!(disabled.status, Status::Disabled);
        let right = svc.authenticate_local("ivan", &pw(PASSWORD)).await;
        let wrong = svc
            .authenticate_local("ivan", &pw("wrong-password-1"))
            .await;
        assert!(matches!(right, Err(AuthError::Disabled)), "{right:?}");
        assert!(
            matches!(wrong, Err(AuthError::InvalidCredentials)),
            "{wrong:?}"
        );

        svc.set_status(ivan.id, Status::Active).await.unwrap();
        assert!(svc.authenticate_local("ivan", &pw(PASSWORD)).await.is_ok());
    })
    .await;
}

#[tokio::test]
async fn login_rehashes_a_hash_with_outdated_params() {
    each_migrated_backend(async |db: &TestDb| {
        let old = service_with(db, FAST).await;
        let judy = old.create(local("judy")).await.unwrap();
        let before = row(db, judy.id).await.password_hash.unwrap();
        assert!(!before.starts_with(DEFAULT_PREFIX));

        let current = service_with(db, HashParams::DEFAULT).await;
        current
            .authenticate_local("judy", &pw(PASSWORD))
            .await
            .unwrap();

        let after = row(db, judy.id).await.password_hash.unwrap();
        assert!(after.starts_with(DEFAULT_PREFIX), "{after}");
        current
            .authenticate_local("judy", &pw(PASSWORD))
            .await
            .unwrap();
        assert_eq!(row(db, judy.id).await.password_hash.unwrap(), after);
        let wrong = current
            .authenticate_local("judy", &pw("wrong-password-1"))
            .await;
        assert!(matches!(wrong, Err(AuthError::InvalidCredentials)));
    })
    .await;
}

#[tokio::test]
async fn set_password_replaces_the_hash_and_checks_policy() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;
        let ken = svc.create(local("ken")).await.unwrap();

        let weak = svc.set_password(ken.id, &pw("short"), false).await;
        assert!(
            matches!(weak, Err(UserError::PolicyViolation { .. })),
            "{weak:?}"
        );
        svc.set_password(ken.id, &pw("new-Secret-Phrase"), true)
            .await
            .unwrap();

        let old = svc.authenticate_local("ken", &pw(PASSWORD)).await;
        assert!(matches!(old, Err(AuthError::InvalidCredentials)));
        let user = svc
            .authenticate_local("ken", &pw("new-Secret-Phrase"))
            .await
            .unwrap();
        assert!(user.user.must_change_password);
        let missing = svc
            .set_password(9999, &pw("new-Secret-Phrase"), false)
            .await;
        assert!(matches!(missing, Err(UserError::NotFound)));
    })
    .await;
}

#[tokio::test]
async fn the_last_active_owner_is_protected() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;
        let a = svc
            .create(with_role(local("owner-a"), Role::Owner))
            .await
            .unwrap();
        let disabled_owner = NewUser {
            status: Status::Disabled,
            ..with_role(local("owner-c"), Role::Owner)
        };
        svc.create(disabled_owner).await.unwrap();

        let demote = svc.set_role(a.id, Role::Admin).await;
        let disable = svc.set_status(a.id, Status::Disabled).await;
        let delete = svc.delete(a.id).await;
        assert!(matches!(demote, Err(UserError::LastOwner)), "{demote:?}");
        assert!(matches!(disable, Err(UserError::LastOwner)), "{disable:?}");
        assert!(matches!(delete, Err(UserError::LastOwner)), "{delete:?}");
        assert_eq!(svc.get(a.id).await.unwrap().role, Role::Owner);

        let b = svc
            .create(with_role(local("owner-b"), Role::Owner))
            .await
            .unwrap();
        assert_eq!(
            svc.set_role(a.id, Role::Admin).await.unwrap().role,
            Role::Admin
        );
        let last = svc.set_role(b.id, Role::User).await;
        assert!(matches!(last, Err(UserError::LastOwner)), "{last:?}");
        // Promoting and re-demoting with another owner present is fine.
        svc.set_role(a.id, Role::Owner).await.unwrap();
        svc.delete(b.id).await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_demotions_leave_exactly_one_owner() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;
        let a = svc
            .create(with_role(local("owner-a"), Role::Owner))
            .await
            .unwrap();
        let b = svc
            .create(with_role(local("owner-b"), Role::Owner))
            .await
            .unwrap();

        let (ra, rb) = tokio::join!(svc.set_role(a.id, Role::Admin), svc.delete(b.id));

        let outcomes = [ra.map(drop), rb];
        let refused = outcomes
            .iter()
            .filter(|r| matches!(r, Err(UserError::LastOwner)))
            .count();
        let succeeded = outcomes.iter().filter(|r| r.is_ok()).count();
        assert_eq!((succeeded, refused), (1, 1), "{outcomes:?}");
    })
    .await;
}

#[tokio::test]
async fn delete_removes_the_user_and_cascades_memberships() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;
        let groups = GroupService::new(db.conn.clone());
        let leo = svc.create(local("leo")).await.unwrap();
        let group = groups.create(NewGroup::new("ops")).await.unwrap();
        groups.add_member(group.id, leo.id).await.unwrap();

        svc.delete(leo.id).await.unwrap();

        assert!(matches!(svc.get(leo.id).await, Err(UserError::NotFound)));
        assert!(groups.members(group.id).await.unwrap().is_empty());
        assert!(matches!(svc.delete(leo.id).await, Err(UserError::NotFound)));
    })
    .await;
}

async fn insert_book(
    db: &TestDb,
    owner: i64,
    kind: address_book::Kind,
    guid: &str,
) -> address_book::Model {
    let at = time::from_millis(1_700_000_000_123).unwrap();
    let book = address_book::ActiveModel {
        guid: Set(guid.to_owned()),
        kind: Set(kind),
        owner_user_id: Set(Some(owner)),
        name: Set(guid.to_owned()),
        note: Set(String::new()),
        created_at: Set(at),
        updated_at: Set(at),
        ..Default::default()
    };
    let book = book.insert(&db.conn).await.unwrap();
    let peer = address_book_peer::ActiveModel {
        address_book_id: Set(book.id),
        rustdesk_id: Set("123".to_owned()),
        alias: Set(String::new()),
        note: Set(String::new()),
        tags: Set("[]".to_owned()),
        hash_enc: Set(None),
        password_enc: Set(Some("v1:k1:nonce:ct".to_owned())),
        username: Set(String::new()),
        hostname: Set(String::new()),
        platform: Set(String::new()),
        force_always_relay: Set(false),
        rdp_port: Set(String::new()),
        rdp_username: Set(String::new()),
        login_name: Set(String::new()),
        extra: Set("{}".to_owned()),
        updated_at: Set(at),
    };
    peer.insert(&db.conn).await.unwrap();
    book
}

#[tokio::test]
async fn delete_removes_the_personal_book_and_unowns_shared_books() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;
        let leo = svc.create(local("leo")).await.unwrap();
        let mia = svc.create(local("mia")).await.unwrap();
        insert_book(db, leo.id, address_book::Kind::Personal, "p-leo").await;
        let shared = insert_book(db, leo.id, address_book::Kind::Shared, "s-leo").await;
        let other = insert_book(db, mia.id, address_book::Kind::Personal, "p-mia").await;

        svc.delete(leo.id).await.unwrap();

        let b = db.backend;
        let mut books = address_book::Entity::find().all(&db.conn).await.unwrap();
        books.sort_by_key(|book| book.id);
        let kept: Vec<_> = books
            .iter()
            .map(|book| (book.guid.as_str(), book.owner_user_id))
            .collect();
        assert_eq!(
            kept,
            vec![("s-leo", None), ("p-mia", Some(mia.id))],
            "{b}: personal book removed, shared unowned, others untouched"
        );
        let peers = address_book_peer::Entity::find()
            .all(&db.conn)
            .await
            .unwrap();
        let mut peer_books: Vec<_> = peers.iter().map(|p| p.address_book_id).collect();
        peer_books.sort();
        assert_eq!(peer_books, vec![shared.id, other.id], "{b}: peers");
    })
    .await;
}

#[tokio::test]
async fn disable_delete_and_password_change_emit_events() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;
        let mut events = svc.subscribe();
        let mia = svc.create(local("mia")).await.unwrap();
        assert!(matches!(events.try_recv(), Err(TryRecvError::Empty)));

        svc.set_status(mia.id, Status::Disabled).await.unwrap();
        assert_eq!(
            events.try_recv().unwrap(),
            UserEvent::Disabled { user_id: mia.id }
        );
        svc.set_status(mia.id, Status::Disabled).await.unwrap();
        svc.set_status(mia.id, Status::Active).await.unwrap();
        assert!(matches!(events.try_recv(), Err(TryRecvError::Empty)));

        svc.set_password(mia.id, &pw("another-Phrase-7"), false)
            .await
            .unwrap();
        assert_eq!(
            events.try_recv().unwrap(),
            UserEvent::PasswordChanged { user_id: mia.id }
        );

        svc.delete(mia.id).await.unwrap();
        let deleted = events.try_recv().unwrap();
        assert_eq!(deleted, UserEvent::Deleted { user_id: mia.id });
        assert_eq!(deleted.user_id(), mia.id);
        assert!(matches!(events.try_recv(), Err(TryRecvError::Empty)));
    })
    .await;
}

#[tokio::test]
async fn failed_operations_emit_no_event() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;
        let owner = svc
            .create(with_role(local("solo"), Role::Owner))
            .await
            .unwrap();
        let mut events = svc.subscribe();

        assert!(svc.set_status(owner.id, Status::Disabled).await.is_err());
        assert!(svc.delete(owner.id).await.is_err());
        assert!(
            svc.set_password(owner.id, &pw("short"), false)
                .await
                .is_err()
        );

        assert!(matches!(events.try_recv(), Err(TryRecvError::Empty)));
    })
    .await;
}

#[tokio::test]
async fn update_profile_changes_fields_and_resets_verification_on_new_email() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;
        let nina = NewUser {
            email: Some("nina@example.com".to_owned()),
            email_verified: true,
            ..local("nina")
        };
        let nina = svc.create(nina).await.unwrap();
        let other = NewUser {
            email: Some("other@example.com".to_owned()),
            ..local("other")
        };
        svc.create(other).await.unwrap();

        let renamed = ProfileUpdate {
            display_name: Some("Nina N.".to_owned()),
            note: Some("on call".to_owned()),
            ..ProfileUpdate::default()
        };
        let updated = svc.update_profile(nina.id, renamed).await.unwrap();
        assert_eq!(updated.display_name, "Nina N.");
        assert_eq!(updated.note, "on call");
        assert!(updated.email_verified);

        let same_email_new_case = ProfileUpdate {
            email: Some(Some("NINA@example.com".to_owned())),
            ..ProfileUpdate::default()
        };
        let updated = svc
            .update_profile(nina.id, same_email_new_case)
            .await
            .unwrap();
        assert_eq!(updated.email.as_deref(), Some("NINA@example.com"));
        assert!(updated.email_verified);

        let moved = ProfileUpdate {
            email: Some(Some("nina@corp.example".to_owned())),
            ..ProfileUpdate::default()
        };
        let updated = svc.update_profile(nina.id, moved).await.unwrap();
        assert!(!updated.email_verified);

        let taken = ProfileUpdate {
            email: Some(Some("Other@Example.com".to_owned())),
            ..ProfileUpdate::default()
        };
        let taken = svc.update_profile(nina.id, taken).await;
        assert!(
            matches!(taken, Err(UserError::Conflict { field: "email" })),
            "{taken:?}"
        );

        let cleared = ProfileUpdate {
            email: Some(None),
            ..ProfileUpdate::default()
        };
        let updated = svc.update_profile(nina.id, cleared).await.unwrap();
        assert_eq!(updated.email, None);
        assert_eq!(row(db, nina.id).await.email_norm, None);

        let bad = ProfileUpdate {
            display_name: Some("line\nbreak".to_owned()),
            ..ProfileUpdate::default()
        };
        let bad = svc.update_profile(nina.id, bad).await;
        assert!(matches!(
            bad,
            Err(UserError::Invalid {
                field: "display_name",
                ..
            })
        ));
        let missing = svc.update_profile(9999, ProfileUpdate::default()).await;
        assert!(matches!(missing, Err(UserError::NotFound)));
    })
    .await;
}

fn names(users: &[User]) -> Vec<&str> {
    users.iter().map(|u| u.username.as_str()).collect()
}

#[tokio::test]
async fn list_paginates_and_filters() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;
        svc.create(with_role(local("owner"), Role::Owner))
            .await
            .unwrap();
        svc.create(with_role(local("admin1"), Role::Admin))
            .await
            .unwrap();
        let with_email = NewUser {
            email: Some("ops_team@example.com".to_owned()),
            display_name: "Ops Person".to_owned(),
            ..local("user1")
        };
        svc.create(with_email).await.unwrap();
        let disabled = NewUser {
            status: Status::Disabled,
            ..local("user2")
        };
        svc.create(disabled).await.unwrap();
        svc.create(local("user3")).await.unwrap();

        let page = |page, page_size| PageRequest { page, page_size };
        let all = svc
            .list(&UserQuery {
                page: page(1, 2),
                ..UserQuery::default()
            })
            .await
            .unwrap();
        assert_eq!(all.total, 5);
        assert_eq!(names(&all.items), ["owner", "admin1"]);
        let last = svc
            .list(&UserQuery {
                page: page(3, 2),
                ..UserQuery::default()
            })
            .await
            .unwrap();
        assert_eq!(names(&last.items), ["user3"]);

        let query = |q: Option<&str>, role, status| UserQuery {
            q: q.map(str::to_owned),
            role,
            status,
            page: page(1, 50),
            ..UserQuery::default()
        };
        let users = svc
            .list(&query(None, Some(Role::User), None))
            .await
            .unwrap();
        assert_eq!(names(&users.items), ["user1", "user2", "user3"]);
        let disabled = svc
            .list(&query(None, None, Some(Status::Disabled)))
            .await
            .unwrap();
        assert_eq!(names(&disabled.items), ["user2"]);
        let by_name = svc.list(&query(Some("ADMIN"), None, None)).await.unwrap();
        assert_eq!(names(&by_name.items), ["admin1"]);
        let by_email = svc
            .list(&query(Some("ops_team"), None, None))
            .await
            .unwrap();
        assert_eq!(names(&by_email.items), ["user1"]);
        let by_display = svc
            .list(&query(Some("ops person"), None, None))
            .await
            .unwrap();
        assert_eq!(names(&by_display.items), ["user1"]);
        // LIKE wildcards in `q` are literal.
        let wildcard = svc.list(&query(Some("%"), None, None)).await.unwrap();
        assert_eq!(wildcard.total, 0);
        let underscore = svc.list(&query(Some("s_team"), None, None)).await.unwrap();
        assert_eq!(names(&underscore.items), ["user1"]);
        let no_match = svc.list(&query(Some("er_"), None, None)).await.unwrap();
        assert_eq!(no_match.total, 0);

        let invalid = svc
            .list(&UserQuery {
                page: page(0, 10),
                ..UserQuery::default()
            })
            .await;
        assert!(matches!(
            invalid,
            Err(UserError::Invalid { field: "page", .. })
        ));
        let too_big = svc
            .list(&UserQuery {
                page: page(1, 10_000),
                ..UserQuery::default()
            })
            .await;
        assert!(matches!(
            too_big,
            Err(UserError::Invalid {
                field: "page_size",
                ..
            })
        ));
    })
    .await;
}

#[tokio::test]
async fn unknown_users_are_not_found() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;

        assert!(matches!(svc.get(42).await, Err(UserError::NotFound)));
        assert!(matches!(
            svc.get_by_username("ghost").await,
            Err(UserError::NotFound)
        ));
        assert!(matches!(
            svc.get_by_username("not valid").await,
            Err(UserError::NotFound)
        ));
        assert!(matches!(
            svc.set_role(42, Role::Admin).await,
            Err(UserError::NotFound)
        ));
        assert!(matches!(
            svc.set_status(42, Status::Disabled).await,
            Err(UserError::NotFound)
        ));
    })
    .await;
}
