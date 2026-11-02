//! Guarded (console) writes: a `TxGuard` refuses inside the write transaction (nothing
//! changes, no audit row) or records its audit row with the change; the combined account
//! edit; batch creation; temporary passwords; user and group listing for the console.
//! Every test runs on SQLite and PostgreSQL.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;

use deskhaven_config::settings::PinSet;
use deskhaven_core::auth::limiter::{Unlocked, unlock_guarded};
use deskhaven_core::clock::SystemClock;
use deskhaven_core::console_audit::{ACTOR_INACTIVE, Actor, NewConsoleAudit, Refused, TxGuard};
use deskhaven_core::crypto::{MasterKey, SecretBox};
use deskhaven_core::groups::{
    GroupError, GroupQuery, GroupSort, GroupSortField, GroupUpdate, Membership, NewGroup,
};
use deskhaven_core::pagination::PageRequest;
use deskhaven_core::sessions::{ClientDevice, SessionError, SessionKind};
use deskhaven_core::settings::SettingsService;
use deskhaven_core::state::{AppState, StateConfig};
use deskhaven_core::tfa::TfaError;
use deskhaven_core::users::{
    AccountEdit, AuthSource, BatchError, Credential, HashLimits, HashParams, NewUser, Password,
    PasswordHasher, ProfileUpdate, Role, Status, TEMPORARY_PASSWORD_LEN, User, UserError,
    UserQuery, UserService, UserSort, UserSortField, check_password_policy,
};
use deskhaven_db::entity::console_audit;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{EntityTrait, PaginatorTrait};
use serde_json::{Value, json};

const PASSWORD: &str = "tundra-Lamp-42";
const FAST: HashParams = HashParams {
    memory_kib: 1024,
    iterations: 1,
    parallelism: 1,
};

async fn state(db: &TestDb) -> AppState {
    let crypto = Arc::new(SecretBox::new(&MasterKey::from_bytes(&[7; 32])));
    let (settings, _) =
        SettingsService::load(db.conn.clone(), Arc::clone(&crypto), PinSet::empty())
            .await
            .unwrap();
    AppState::new(
        db.conn.clone(),
        settings,
        crypto,
        "key".to_owned(),
        PasswordHasher::new(FAST).unwrap(),
        Arc::new(SystemClock),
        &StateConfig::default(),
    )
}

fn local(username: &str, role: Role) -> NewUser {
    let credential = Credential::Local {
        password: Password::new(PASSWORD),
    };
    NewUser {
        role,
        ..NewUser::new(username, credential)
    }
}

async fn user(state: &AppState, username: &str, role: Role) -> User {
    state.users.create(local(username, role)).await.unwrap()
}

fn snapshot<T: std::fmt::Debug>(value: Option<&T>) -> Option<Value> {
    value.map(|v| json!(format!("{v:?}")))
}

/// Records `action` with debug snapshots of before / after.
fn recording<T: std::fmt::Debug + 'static>(state: &AppState, action: &str) -> TxGuard<T> {
    let action = action.to_owned();
    TxGuard::new(&state.console_audit, move |before, after| {
        Ok(NewConsoleAudit {
            actor: Actor::User(1),
            action,
            target_type: "test".to_owned(),
            target_id: String::new(),
            before: snapshot(before),
            after: snapshot(after),
            ip: None,
        })
    })
}

fn is_refused<T>(result: Result<T, UserError>) -> bool {
    matches!(result, Err(UserError::Refused(_)))
}

fn refusing<T: 'static>(state: &AppState) -> TxGuard<T> {
    TxGuard::new(&state.console_audit, |_, _| Err(Refused::new("nope")))
}

async fn audit_rows(db: &TestDb) -> Vec<console_audit::Model> {
    console_audit::Entity::find().all(&db.conn).await.unwrap()
}

#[tokio::test]
async fn refused_user_writes_change_nothing_and_record_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        let state = state(db).await;
        let _owner = user(&state, "owner", Role::Owner).await;
        let bob = user(&state, "bob", Role::User).await;

        let edit = AccountEdit {
            role: Some(Role::Admin),
            status: Some(Status::Disabled),
            ..AccountEdit::default()
        };
        assert!(is_refused(
            state.users.edit(bob.id, edit, refusing(&state)).await
        ));
        let pw = Password::new("another-Strong-pass-9");
        let set = state
            .users
            .set_password_guarded(bob.id, &pw, true, refusing(&state));
        assert!(is_refused(set.await));
        assert!(is_refused(
            state.users.delete_guarded(bob.id, refusing(&state)).await
        ));
        let create = state
            .users
            .create_guarded(local("carol", Role::User), refusing(&state));
        assert!(is_refused(create.await));

        assert_eq!(state.users.get(bob.id).await.unwrap(), bob);
        assert!(state.users.get_by_username("carol").await.is_err());
        assert!(audit_rows(db).await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn guarded_user_writes_record_before_and_after_in_their_transaction() {
    each_migrated_backend(async |db: &TestDb| {
        let state = state(db).await;
        let _owner = user(&state, "owner", Role::Owner).await;
        let carol = state
            .users
            .create_guarded(local("carol", Role::User), recording(&state, "create"))
            .await
            .unwrap();
        let edit = AccountEdit {
            profile: ProfileUpdate {
                display_name: Some("Carol C".to_owned()),
                ..ProfileUpdate::default()
            },
            role: Some(Role::Admin),
            status: Some(Status::Disabled),
        };
        let edited = state
            .users
            .edit(carol.id, edit, recording(&state, "edit"))
            .await
            .unwrap();
        assert_eq!(
            (edited.display_name.as_str(), edited.role, edited.status),
            ("Carol C", Role::Admin, Status::Disabled)
        );
        let pw = Password::new("another-Strong-pass-9");
        let set = recording(&state, "password");
        state
            .users
            .set_password_guarded(carol.id, &pw, true, set)
            .await
            .unwrap();
        assert!(
            state
                .users
                .get(carol.id)
                .await
                .unwrap()
                .must_change_password
        );
        state
            .users
            .delete_guarded(carol.id, recording(&state, "delete"))
            .await
            .unwrap();

        let rows = audit_rows(db).await;
        let shape: Vec<_> = rows
            .iter()
            .map(|r| (r.action.as_str(), r.before.is_some(), r.after.is_some()))
            .collect();
        assert_eq!(
            shape,
            [
                ("create", false, true),
                ("edit", true, true),
                ("password", true, true),
                ("delete", true, false)
            ]
        );
        let edit_row = &rows[1];
        assert!(edit_row.before.as_deref().unwrap().contains("role: User"));
        assert!(edit_row.after.as_deref().unwrap().contains("role: Admin"));
    })
    .await;
}

#[tokio::test]
async fn edit_keeps_the_last_owner_and_reports_conflicts() {
    each_migrated_backend(async |db: &TestDb| {
        let state = state(db).await;
        let owner = user(&state, "owner", Role::Owner).await;
        let bob = user(&state, "bob", Role::User).await;
        let demote = AccountEdit {
            role: Some(Role::Admin),
            ..AccountEdit::default()
        };
        let got = state
            .users
            .edit(owner.id, demote, recording(&state, "x"))
            .await;
        assert!(matches!(got, Err(UserError::LastOwner)));
        // The audit row written before the refusal rolled back with it.
        assert!(audit_rows(db).await.is_empty());

        state
            .users
            .update_profile(
                owner.id,
                ProfileUpdate {
                    email: Some(Some("o@example.com".to_owned())),
                    ..ProfileUpdate::default()
                },
            )
            .await
            .unwrap();
        let steal = AccountEdit {
            profile: ProfileUpdate {
                email: Some(Some("O@Example.com".to_owned())),
                ..ProfileUpdate::default()
            },
            ..AccountEdit::default()
        };
        let got = state
            .users
            .edit(bob.id, steal, recording(&state, "steal"))
            .await;
        assert!(matches!(got, Err(UserError::Conflict { field: "email" })));
        assert!(audit_rows(db).await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn batch_creation_reports_every_bad_row_and_creates_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        let state = state(db).await;
        user(&state, "taken", Role::User).await;
        let rows = vec![
            local("fine", Role::User),
            local("bad name", Role::User),
            local("Taken", Role::User),
            local("dup", Role::User),
            local("DUP", Role::User),
        ];
        let guarded = rows
            .into_iter()
            .map(|r| (r, recording(&state, "import")))
            .collect();
        let Err(BatchError::Rows(errors)) = state.users.create_batch(guarded).await else {
            panic!("expected row errors");
        };
        let got: Vec<_> = errors.iter().map(|e| (e.index, kind(&e.error))).collect();
        let got: Vec<_> = got.iter().map(|(i, k)| (*i, k.as_str())).collect();
        assert_eq!(
            got,
            [
                (1, "invalid:username"),
                (2, "conflict:username"),
                (4, "conflict:username")
            ]
        );
        assert!(state.users.get_by_username("fine").await.is_err());
        assert!(audit_rows(db).await.is_empty());

        let refused = vec![
            (local("one", Role::User), recording(&state, "import")),
            (local("two", Role::Owner), refusing(&state)),
        ];
        let Err(BatchError::Rows(errors)) = state.users.create_batch(refused).await else {
            panic!("expected the refusal");
        };
        assert_eq!(errors.len(), 1);
        assert_eq!(
            (errors[0].index, kind(&errors[0].error).as_str()),
            (1, "refused")
        );
        assert!(state.users.get_by_username("one").await.is_err());
        assert!(audit_rows(db).await.is_empty());
    })
    .await;
}

fn kind(error: &UserError) -> String {
    match error {
        UserError::Invalid { field, .. } => format!("invalid:{field}"),
        UserError::Conflict { field } => format!("conflict:{field}"),
        UserError::Refused(_) => "refused".to_owned(),
        other => format!("{other:?}"),
    }
}

#[tokio::test]
async fn batch_creation_creates_all_rows_with_one_audit_row_each() {
    each_migrated_backend(async |db: &TestDb| {
        let state = state(db).await;
        let rows = ["a1", "a2", "a3"]
            .map(|name| (local(name, Role::User), recording(&state, "import")))
            .into_iter()
            .collect();
        let created = state.users.create_batch(rows).await.unwrap();
        let names: Vec<_> = created.iter().map(|u| u.username.as_str()).collect();
        assert_eq!(names, ["a1", "a2", "a3"]);
        assert_eq!(audit_rows(db).await.len(), 3);
    })
    .await;
}

#[tokio::test]
async fn batch_creation_hashes_concurrently_and_keeps_each_rows_password() {
    each_migrated_backend(async |db: &TestDb| {
        let state = state(db).await;
        // Three slots: two hashes at a time, so eight rows refill the window repeatedly.
        let limits = HashLimits {
            max_concurrent: 3,
            acquire_timeout: std::time::Duration::from_secs(5),
        };
        let hasher = PasswordHasher::with_limits(FAST, limits).unwrap();
        let users = UserService::new(db.conn.clone(), state.settings.clone(), hasher);
        let password = |name: &str| Password::new(format!("pw-{name}-Strong-9"));
        let names = ["b1", "b2", "b3", "dir", "b4", "b5", "b6", "b7"];
        let rows = names
            .map(|name| {
                let credential = if name == "dir" {
                    Credential::Ldap
                } else {
                    Credential::Local {
                        password: password(name),
                    }
                };
                (NewUser::new(name, credential), recording(&state, "import"))
            })
            .into_iter()
            .collect();
        let created = users.create_batch(rows).await.unwrap();
        assert_eq!(created.len(), names.len());
        for name in names.iter().filter(|n| **n != "dir") {
            let own = users.authenticate_local(name, &password(name)).await;
            assert!(own.is_ok(), "{name}");
            let other = users.authenticate_local(name, &password("b1x")).await;
            assert!(other.is_err(), "{name}");
        }
        let dir = users.get_by_username("dir").await.unwrap();
        assert_eq!(dir.auth_source, AuthSource::Ldap);
    })
    .await;
}

#[tokio::test]
async fn temporary_passwords_pass_the_policy_and_differ() {
    each_migrated_backend(async |db: &TestDb| {
        let state = state(db).await;
        let first = state.users.temporary_password().unwrap();
        let second = state.users.temporary_password().unwrap();
        assert_eq!(first.char_len(), TEMPORARY_PASSWORD_LEN);
        assert_ne!(first.expose(), second.expose());
        assert!(check_password_policy(&first, 10).is_ok());
    })
    .await;
}

#[tokio::test]
async fn user_list_filters_by_source_and_group_and_sorts() {
    each_migrated_backend(async |db: &TestDb| {
        let state = state(db).await;
        let alice = user(&state, "alice", Role::Admin).await;
        let bob = user(&state, "bob", Role::User).await;
        let ldap = NewUser::new("carol", Credential::Ldap);
        let carol = state.users.create(ldap).await.unwrap();
        let group = state.groups.create(NewGroup::new("ops")).await.unwrap();
        state.groups.add_member(group.id, bob.id).await.unwrap();
        state.groups.add_member(group.id, carol.id).await.unwrap();
        let names = async |query: UserQuery| -> Vec<String> {
            let page = state.users.list(&query).await.unwrap();
            page.items.into_iter().map(|u| u.username).collect()
        };
        let base = UserQuery {
            page: PageRequest::default(),
            ..UserQuery::default()
        };
        let ldap_only = UserQuery {
            auth_source: Some(AuthSource::Ldap),
            ..base.clone()
        };
        assert_eq!(names(ldap_only).await, ["carol"]);
        let members = UserQuery {
            group_id: Some(group.id),
            ..base.clone()
        };
        assert_eq!(names(members).await, ["bob", "carol"]);
        let by_name_desc = UserQuery {
            sort: Some(UserSort {
                field: UserSortField::Username,
                descending: true,
            }),
            ..base.clone()
        };
        assert_eq!(names(by_name_desc).await, ["carol", "bob", "alice"]);
        let by_role = UserQuery {
            sort: Some(UserSort {
                field: UserSortField::Role,
                descending: false,
            }),
            ..base
        };
        // Text order of the stored role, then id.
        assert_eq!(names(by_role).await, ["alice", "bob", "carol"]);
        assert!(alice.id < bob.id);
    })
    .await;
}

#[tokio::test]
async fn guarded_group_writes_record_and_refusals_roll_back() {
    each_migrated_backend(async |db: &TestDb| {
        let state = state(db).await;
        let bob = user(&state, "bob", Role::User).await;
        let groups = &state.groups;
        let refused = groups
            .create_guarded(NewGroup::new("ops"), refusing(&state))
            .await;
        assert!(matches!(refused, Err(GroupError::Refused(_))));
        let ops = groups
            .create_guarded(NewGroup::new("ops"), recording(&state, "create"))
            .await
            .unwrap();
        let update = GroupUpdate {
            note: Some("night shift".to_owned()),
            ..GroupUpdate::default()
        };
        groups
            .update_guarded(ops.id, update, recording(&state, "update"))
            .await
            .unwrap();
        let add = |g: TxGuard<Membership>| groups.add_member_guarded(ops.id, bob.id, g);
        add(recording(&state, "add")).await.unwrap();
        add(recording(&state, "add-again")).await.unwrap();
        let remove = |g: TxGuard<Membership>| groups.remove_member_guarded(ops.id, bob.id, g);
        assert!(matches!(
            remove(refusing(&state)).await,
            Err(GroupError::Refused(_))
        ));
        remove(recording(&state, "remove")).await.unwrap();
        groups
            .delete_guarded(ops.id, recording(&state, "delete"))
            .await
            .unwrap();

        let rows = audit_rows(db).await;
        let shape: Vec<_> = rows
            .iter()
            .map(|r| (r.action.as_str(), r.before.is_some(), r.after.is_some()))
            .collect();
        assert_eq!(
            shape,
            [
                ("create", false, true),
                ("update", true, true),
                ("add", false, true),
                ("add-again", true, true),
                ("remove", true, false),
                ("delete", true, false)
            ]
        );
    })
    .await;
}

#[tokio::test]
async fn group_pages_count_members_filter_and_sort() {
    each_migrated_backend(async |db: &TestDb| {
        let state = state(db).await;
        let bob = user(&state, "bob", Role::User).await;
        let carol = user(&state, "carol", Role::User).await;
        let mut ids = Vec::new();
        for name in ["ops", "dev", "ops-night"] {
            ids.push(state.groups.create(NewGroup::new(name)).await.unwrap().id);
        }
        for member in [bob.id, carol.id] {
            state.groups.add_member(ids[0], member).await.unwrap();
        }
        let page = |q: Option<&str>, sort| GroupQuery {
            q: q.map(str::to_owned),
            sort,
            page: PageRequest::default(),
        };
        let all = state.groups.page(&page(None, None)).await.unwrap();
        let listed: Vec<_> = all
            .items
            .iter()
            .map(|s| (s.group.name.as_str(), s.member_count))
            .collect();
        assert_eq!(listed, [("dev", 0), ("ops", 2), ("ops-night", 0)]);
        let ops = state.groups.page(&page(Some("OPS"), None)).await.unwrap();
        assert_eq!(ops.total, 2);
        let newest = GroupSort {
            field: GroupSortField::CreatedAt,
            descending: true,
        };
        let sorted = state.groups.page(&page(None, Some(newest))).await.unwrap();
        assert_eq!(sorted.items[0].group.name, "ops-night");
        let summary = state.groups.summary(ids[0]).await.unwrap();
        assert_eq!(summary.member_count, 2);
    })
    .await;
}

#[tokio::test]
async fn guarded_2fa_reset_session_revocation_and_unlock() {
    each_migrated_backend(async |db: &TestDb| {
        let state = state(db).await;
        let bob = user(&state, "bob", Role::User).await;
        state.tfa.begin_enrolment(bob.id).await.unwrap();

        let refused = state.tfa.disable_guarded(bob.id, refusing(&state)).await;
        assert!(matches!(refused, Err(TfaError::Refused(_))));
        assert!(state.tfa.status(bob.id).await.unwrap().pending_enrolment);
        state
            .tfa
            .disable_guarded(bob.id, recording(&state, "2fa"))
            .await
            .unwrap();
        assert!(!state.tfa.status(bob.id).await.unwrap().pending_enrolment);

        let auth = state
            .users
            .authenticate_local("bob", &Password::new(PASSWORD))
            .await
            .unwrap();
        let device = ClientDevice {
            rustdesk_id: "123456789".to_owned(),
            uuid: "uuid-1".to_owned(),
            info: None,
        };
        let (_, token) = state
            .sessions
            .create_client(bob.id, &auth.stamp, &device, None)
            .await
            .unwrap();
        let sessions = &state.sessions;
        let refused = sessions
            .revoke_all_for_user_guarded(bob.id, refusing(&state))
            .await;
        assert!(matches!(refused, Err(SessionError::Refused(_))));
        assert!(
            sessions
                .authenticate(token.expose(), SessionKind::Client)
                .await
                .is_ok()
        );
        let revoked = sessions
            .revoke_all_for_user_guarded(bob.id, recording(&state, "revoke"))
            .await
            .unwrap();
        assert_eq!(revoked, 1);
        assert!(
            sessions
                .authenticate(token.expose(), SessionKind::Client)
                .await
                .is_err()
        );
        let unknown = sessions
            .revoke_all_for_user_guarded(9999, recording(&state, "x"))
            .await;
        assert!(matches!(unknown, Err(SessionError::UserNotFound)));

        let guard = recording::<Unlocked>(&state, "unlock");
        assert_eq!(unlock_guarded(&db.conn, "bob", guard).await.unwrap(), 0);

        let actions: Vec<_> = audit_rows(db).await.into_iter().map(|r| r.action).collect();
        assert_eq!(actions, ["2fa", "revoke", "unlock"]);
        let count = console_audit::Entity::find().count(&db.conn).await.unwrap();
        assert_eq!(count, 3);
    })
    .await;
}

/// Allows admins and owners (the console's rule), recording `action` as `actor_id`.
fn as_admin<T: 'static>(state: &AppState, actor_id: i64, action: &str) -> TxGuard<T> {
    let action = action.to_owned();
    TxGuard::for_account(&state.console_audit, actor_id, move |role, _, _| {
        if role == Role::User {
            return Err(Refused::new("forbidden"));
        }
        Ok(NewConsoleAudit {
            actor: Actor::User(actor_id),
            action,
            target_type: "test".to_owned(),
            target_id: String::new(),
            before: None,
            after: None,
            ip: None,
        })
    })
}

fn refused_code<T>(result: Result<T, UserError>) -> Option<&'static str> {
    match result {
        Err(UserError::Refused(r)) => Some(r.code),
        _ => None,
    }
}

fn rename(name: &str) -> AccountEdit {
    AccountEdit {
        profile: ProfileUpdate {
            display_name: Some(name.to_owned()),
            ..ProfileUpdate::default()
        },
        ..AccountEdit::default()
    }
}

#[tokio::test]
async fn account_guards_use_the_actors_committed_role_and_status() {
    each_migrated_backend(async |db: &TestDb| {
        let state = state(db).await;
        let _owner = user(&state, "owner", Role::Owner).await;
        let admin = user(&state, "admin", Role::Admin).await;
        let bob = user(&state, "bob", Role::User).await;

        // Still an admin: allowed and recorded.
        let guard = as_admin(&state, admin.id, "edit");
        state
            .users
            .edit(bob.id, rename("Bob 1"), guard)
            .await
            .unwrap();
        assert_eq!(audit_rows(db).await.len(), 1);

        // Demoted after the request was authorized: the stale principal is refused.
        state.users.set_role(admin.id, Role::User).await.unwrap();
        let guard = as_admin(&state, admin.id, "edit");
        let demoted = state.users.edit(bob.id, rename("Bob 2"), guard).await;
        assert_eq!(refused_code(demoted), Some("forbidden"));

        // Disabled (role kept) or deleted: refused before any rule runs.
        state.users.set_role(admin.id, Role::Admin).await.unwrap();
        state
            .users
            .set_status(admin.id, Status::Disabled)
            .await
            .unwrap();
        let guard = as_admin(&state, admin.id, "edit");
        let disabled = state.users.edit(bob.id, rename("Bob 3"), guard).await;
        assert_eq!(refused_code(disabled), Some(ACTOR_INACTIVE));
        let guard = as_admin(&state, 9_999, "delete");
        let gone = state.users.delete_guarded(bob.id, guard).await;
        assert_eq!(refused_code(gone), Some(ACTOR_INACTIVE));

        assert_eq!(state.users.get(bob.id).await.unwrap().display_name, "Bob 1");
        assert_eq!(audit_rows(db).await.len(), 1);
    })
    .await;
}
