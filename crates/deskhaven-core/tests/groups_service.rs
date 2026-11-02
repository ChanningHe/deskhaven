//! GroupService: CRUD, membership, config-managed groups. Every test runs on SQLite and
//! PostgreSQL.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;

use deskhaven_config::settings::PinSet;
use deskhaven_core::crypto::{MasterKey, SecretBox};
use deskhaven_core::groups::{Group, GroupError, GroupService, GroupUpdate, ManagedBy, NewGroup};
use deskhaven_core::settings::SettingsService;
use deskhaven_core::users::{
    Credential, HashParams, NewUser, Password, PasswordHasher, User, UserService,
};
use deskhaven_db::entity::group;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;
use sea_orm::{ActiveModelTrait, ActiveValue::Set};

const FAST: HashParams = HashParams {
    memory_kib: 1024,
    iterations: 1,
    parallelism: 1,
};

async fn users(db: &TestDb) -> UserService {
    let crypto = Arc::new(SecretBox::new(&MasterKey::from_bytes(&[7; 32])));
    let (settings, _) = SettingsService::load(db.conn.clone(), crypto, PinSet::empty())
        .await
        .unwrap();
    UserService::new(
        db.conn.clone(),
        settings,
        PasswordHasher::new(FAST).unwrap(),
    )
}

async fn user(svc: &UserService, name: &str) -> User {
    let credential = Credential::Local {
        password: Password::new("tundra-Lamp-42"),
    };
    svc.create(NewUser::new(name, credential)).await.unwrap()
}

async fn config_group(db: &TestDb, name: &str) -> group::Model {
    let now = time::now();
    group::ActiveModel {
        name: Set(name.to_owned()),
        note: Set(String::new()),
        managed_by: Set(ManagedBy::Config),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&db.conn)
    .await
    .unwrap()
}

fn group_names(groups: &[Group]) -> Vec<&str> {
    groups.iter().map(|g| g.name.as_str()).collect()
}

fn user_names(users: &[User]) -> Vec<&str> {
    users.iter().map(|u| u.username.as_str()).collect()
}

#[tokio::test]
async fn groups_can_be_created_updated_listed_and_deleted() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = GroupService::new(db.conn.clone());

        let ops = svc.create(NewGroup::new("ops")).await.unwrap();
        let dev = NewGroup {
            note: "developers".to_owned(),
            ..NewGroup::new("dev")
        };
        let dev = svc.create(dev).await.unwrap();
        assert_eq!(ops.managed_by, ManagedBy::Ui);
        assert_eq!(dev.note, "developers");
        let duplicate = svc.create(NewGroup::new("ops")).await;
        assert!(
            matches!(duplicate, Err(GroupError::Conflict { field: "name" })),
            "{duplicate:?}"
        );
        assert_eq!(group_names(&svc.list().await.unwrap()), ["dev", "ops"]);

        let rename = GroupUpdate {
            name: Some("operations".to_owned()),
            note: Some("on call".to_owned()),
        };
        let renamed = svc.update(ops.id, rename).await.unwrap();
        assert_eq!(renamed.name, "operations");
        assert_eq!(renamed.note, "on call");
        assert_eq!(svc.get(ops.id).await.unwrap(), renamed);
        let clash = GroupUpdate {
            name: Some("dev".to_owned()),
            note: None,
        };
        let clash = svc.update(ops.id, clash).await;
        assert!(matches!(clash, Err(GroupError::Conflict { field: "name" })));

        svc.delete(dev.id).await.unwrap();
        assert!(matches!(svc.get(dev.id).await, Err(GroupError::NotFound)));
        assert!(matches!(
            svc.delete(dev.id).await,
            Err(GroupError::NotFound)
        ));
    })
    .await;
}

#[tokio::test]
async fn group_names_are_validated() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = GroupService::new(db.conn.clone());

        for bad in ["", "   ", "tab\there", &"g".repeat(65)] {
            let result = svc.create(NewGroup::new(bad)).await;
            assert!(
                matches!(result, Err(GroupError::Invalid { field: "name", .. })),
                "{bad:?}: {result:?}"
            );
        }
        assert!(svc.create(NewGroup::new("Ops Team / EU")).await.is_ok());
    })
    .await;
}

#[tokio::test]
async fn membership_can_be_added_listed_and_removed() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = GroupService::new(db.conn.clone());
        let people = users(db).await;
        let alice = user(&people, "alice").await;
        let bob = user(&people, "bob").await;
        let ops = svc.create(NewGroup::new("ops")).await.unwrap();
        let dev = svc.create(NewGroup::new("dev")).await.unwrap();

        svc.add_member(ops.id, alice.id).await.unwrap();
        svc.add_member(ops.id, alice.id).await.unwrap();
        svc.add_member(ops.id, bob.id).await.unwrap();
        svc.add_member(dev.id, alice.id).await.unwrap();

        assert_eq!(
            user_names(&svc.members(ops.id).await.unwrap()),
            ["alice", "bob"]
        );
        assert_eq!(
            group_names(&svc.groups_of_user(alice.id).await.unwrap()),
            ["dev", "ops"]
        );
        svc.remove_member(ops.id, alice.id).await.unwrap();
        svc.remove_member(ops.id, alice.id).await.unwrap();
        assert_eq!(user_names(&svc.members(ops.id).await.unwrap()), ["bob"]);
        assert_eq!(
            group_names(&svc.groups_of_user(alice.id).await.unwrap()),
            ["dev"]
        );

        let no_user = svc.add_member(ops.id, 9999).await;
        let no_group = svc.add_member(9999, alice.id).await;
        assert!(
            matches!(no_user, Err(GroupError::UserNotFound)),
            "{no_user:?}"
        );
        assert!(
            matches!(no_group, Err(GroupError::NotFound)),
            "{no_group:?}"
        );
        assert!(matches!(svc.members(9999).await, Err(GroupError::NotFound)));
        assert!(matches!(
            svc.groups_of_user(9999).await,
            Err(GroupError::UserNotFound)
        ));
    })
    .await;
}

#[tokio::test]
async fn deleting_a_group_removes_its_memberships() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = GroupService::new(db.conn.clone());
        let people = users(db).await;
        let carol = user(&people, "carol").await;
        let ops = svc.create(NewGroup::new("ops")).await.unwrap();
        svc.add_member(ops.id, carol.id).await.unwrap();

        svc.delete(ops.id).await.unwrap();

        assert!(svc.groups_of_user(carol.id).await.unwrap().is_empty());
    })
    .await;
}

#[tokio::test]
async fn config_managed_groups_are_read_only() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = GroupService::new(db.conn.clone());
        let people = users(db).await;
        let dave = user(&people, "dave").await;
        let managed = config_group(db, "from-config").await;

        let update = GroupUpdate {
            name: None,
            note: Some("edited".to_owned()),
        };
        let update = svc.update(managed.id, update).await;
        let add = svc.add_member(managed.id, dave.id).await;
        let remove = svc.remove_member(managed.id, dave.id).await;
        let delete = svc.delete(managed.id).await;
        for result in [add, remove, delete] {
            assert!(
                matches!(result, Err(GroupError::ManagedByConfig)),
                "{result:?}"
            );
        }
        assert!(
            matches!(update, Err(GroupError::ManagedByConfig)),
            "{update:?}"
        );

        let read = svc.get(managed.id).await.unwrap();
        assert_eq!(read.managed_by, ManagedBy::Config);
        assert_eq!(read.note, "");
        assert!(svc.members(managed.id).await.unwrap().is_empty());
    })
    .await;
}
