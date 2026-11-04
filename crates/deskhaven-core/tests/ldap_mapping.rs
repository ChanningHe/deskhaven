//! Group mapping and the admin flag of directory accounts (06 §3.5), applied at every
//! login: `memberOf` or a group search, groups below `group_base_dn` only, console-managed
//! groups only, owners never changed. Both backends.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::ldap::Env;
use common::ldap_fake::{FakeUser, SERVICE_DN, USER_BASE, USER_PASSWORD, group_dn};
use deskhaven_core::groups::ManagedBy;
use deskhaven_core::users::{Credential, NewUser, Role};
use deskhaven_db::entity::console_audit::{self, ActorKind};
use deskhaven_db::entity::group;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, EntityTrait, QueryOrder};

#[tokio::test]
async fn group_mapping_follows_the_directory_at_every_login() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let dn = env.fake.add_user("alice");
        env.group("ops").await;
        env.group("dev").await;
        let console_only = env.group("console-only").await;
        env.fake.set_members(&group_dn("ops"), &[&dn]);
        // A group outside group_base_dn is not mapped, even with a matching name.
        env.fake
            .set_members("cn=dev,ou=elsewhere,dc=example,dc=org", &[&dn]);

        let id = env.login("alice", USER_PASSWORD).await.unwrap().user.id;
        let first = env.group_names(id).await;
        env.groups.add_member(console_only, id).await.unwrap();
        env.fake.set_members(&group_dn("ops"), &[]);
        env.fake.set_members(&group_dn("DEV"), &[&dn]);
        env.login("alice", USER_PASSWORD).await.unwrap();
        let second = env.group_names(id).await;

        assert_eq!(first, ["ops"], "{}", db.backend);
        // The directory decides the membership of directory accounts (names compare
        // case-insensitively).
        assert_eq!(second, ["dev"]);
    })
    .await;
}

#[tokio::test]
async fn configuration_managed_groups_are_never_touched() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let dn = env.fake.add_user("alice");
        let managed = env.group("ops").await;
        let mut row: group::ActiveModel = group::Entity::find_by_id(managed)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap()
            .into();
        row.managed_by = Set(ManagedBy::Config);
        row.update(&db.conn).await.unwrap();
        env.fake.set_members(&group_dn("ops"), &[&dn]);

        let id = env.login("alice", USER_PASSWORD).await.unwrap().user.id;

        assert!(env.group_names(id).await.is_empty(), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn no_group_base_dn_leaves_memberships_to_the_console() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |d| d.group_base_dn = None).await;
        let dn = env.fake.add_user("alice");
        let ops = env.group("ops").await;
        env.fake.set_members(&group_dn("dev"), &[&dn]);

        let id = env.login("alice", USER_PASSWORD).await.unwrap().user.id;
        env.groups.add_member(ops, id).await.unwrap();
        env.login("alice", USER_PASSWORD).await.unwrap();

        assert_eq!(env.group_names(id).await, ["ops"], "{}", db.backend);
        // Neither mapping nor an admin group: memberOf is not even asked for.
        assert_eq!(env.fake.searches()[0].attrs, ["uid", "mail", "cn"]);
    })
    .await;
}

#[tokio::test]
async fn group_search_uses_the_filter_template_with_escaped_values() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |d| {
            d.group_filter = Some("(&(objectClass=groupOfNames)(member={user_dn}))".to_owned());
        })
        .await;
        let dn = env.fake.put_user(FakeUser {
            dn: format!("uid=star*,{USER_BASE}"),
            password: USER_PASSWORD.to_owned(),
            attrs: [("uid".to_owned(), vec!["star".to_owned()])].into(),
        });
        env.group("ops").await;
        env.fake.set_members(&group_dn("ops"), &[&dn]);

        let id = env.login("star", USER_PASSWORD).await.unwrap().user.id;

        let searches = env.fake.searches();
        assert_eq!(searches.len(), 2, "{}: {searches:?}", db.backend);
        assert_eq!(
            searches[1].filter,
            format!("(&(objectClass=groupOfNames)(member=uid=star\\2a,{USER_BASE}))")
        );
        assert_eq!(env.group_names(id).await, ["ops"]);
        // Only DNs are wanted: `1.1`, not the empty list (which means all attributes).
        assert_eq!(searches[1].attrs, ["1.1"]);
        // The group search runs as the service account, after the user's bind.
        let binds = env.fake.binds();
        assert_eq!(binds, [SERVICE_DN.to_owned(), dn, SERVICE_DN.to_owned()]);
    })
    .await;
}

#[tokio::test]
async fn the_username_placeholder_is_the_directory_value_that_matched() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |d| {
            d.group_filter = Some("(memberUid={username})".to_owned());
        })
        .await;
        let dn = env.fake.add_user("alice");
        env.fake.set_attr(&dn, "uid", &["a-alias", "alice"]);

        env.login("ALICE", USER_PASSWORD).await.unwrap();

        let searches = env.fake.searches();
        assert_eq!(searches[1].filter, "(memberUid=alice)", "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn directory_changes_to_roles_and_groups_are_audited_as_the_system() {
    each_migrated_backend(async |db: &TestDb| {
        let admins = group_dn("admins");
        let admin_dn = admins.clone();
        let env = Env::with(db, move |d| d.admin_group_dn = Some(admin_dn)).await;
        let dn = env.fake.add_user("alice");
        env.group("ops").await;
        env.fake.set_members(&group_dn("ops"), &[&dn]);
        env.fake.set_members(&admins, &[&dn]);

        env.login("alice", USER_PASSWORD).await.unwrap();
        env.fake.set_members(&group_dn("ops"), &[]);
        env.login("alice", USER_PASSWORD).await.unwrap();
        // Nothing changed: nothing more is audited.
        env.login("alice", USER_PASSWORD).await.unwrap();

        let rows = console_audit::Entity::find()
            .order_by_asc(console_audit::Column::Id)
            .all(&db.conn)
            .await
            .unwrap();
        let actions: Vec<&str> = rows.iter().map(|r| r.action.as_str()).collect();
        assert_eq!(
            actions,
            ["user.update", "group.member_add", "group.member_remove"],
            "{}",
            db.backend
        );
        assert!(rows.iter().all(|r| r.actor_kind == ActorKind::System));
        assert!(rows[0].after.as_deref().unwrap().contains("admin"));
    })
    .await;
}

#[tokio::test]
async fn the_admin_group_promotes_and_demotes_but_never_touches_owners() {
    each_migrated_backend(async |db: &TestDb| {
        let admins = group_dn("deskhaven-admins");
        let admin_dn = admins.clone();
        let env = Env::with(db, move |d| {
            d.admin_group_dn = Some(admin_dn.to_uppercase())
        })
        .await;
        let alice = env.fake.add_user("alice");
        env.fake.add_user("olivia");
        env.fake.set_members(&admins, &[&alice]);
        let owner = NewUser {
            role: Role::Owner,
            ..NewUser::new("olivia", Credential::Ldap)
        };
        env.users.create(owner).await.unwrap();

        let promoted = env.login("alice", USER_PASSWORD).await.unwrap().user;
        env.fake.set_members(&admins, &[]);
        let demoted = env.login("alice", USER_PASSWORD).await.unwrap().user;
        let owner = env.login("olivia", USER_PASSWORD).await.unwrap().user;

        // DNs compare case-insensitively.
        assert_eq!(promoted.role, Role::Admin, "{}", db.backend);
        assert_eq!(demoted.role, Role::User);
        assert_eq!(env.users.get(demoted.id).await.unwrap().role, Role::User);
        assert_eq!(owner.role, Role::Owner);
    })
    .await;
}
