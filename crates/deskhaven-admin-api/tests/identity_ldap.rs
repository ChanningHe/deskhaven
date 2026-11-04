//! LDAP directory administration (M3-09, 06 §3.6): every write is owner-only (whoever can
//! change the URL can impersonate every directory account), admins may read; the bind
//! password is write-only and must be entered again when the connection changes;
//! config-managed directories are read-only (409); writes are audited with url, bind DN,
//! TLS and enabled but never the password. Runs on SQLite and PostgreSQL.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

#[path = "../../deskhaven-core/tests/common/ldap_fake.rs"]
mod ldap_fake;

use axum::http::Method;
use common::login::harness_with_ldap;
use common::tokens::snapshot;
use common::{Console, Harness};
use deskhaven_core::users::Role;
use deskhaven_db::entity::ldap_directory;
use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use ldap_fake::{FakeDirectory, SERVICE_DN, SERVICE_PASSWORD, USER_BASE};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, EntityTrait};
use serde_json::{Value, json};

const DIRECTORIES: &str = "/admin/api/v1/ldap-directories";

fn path(id: i64) -> String {
    format!("{DIRECTORIES}/{id}")
}

struct Env {
    h: Harness,
    owner: Console,
    admin: Console,
    user: Console,
}

async fn setup(db: &TestDb) -> Env {
    let h = harness_with_ldap(db, FakeDirectory::new().connector()).await;
    let owner = h.console("own", Role::Owner).await;
    let admin = h.console("adm", Role::Admin).await;
    let user = h.console("usr", Role::User).await;
    Env {
        h,
        owner,
        admin,
        user,
    }
}

fn corp() -> Value {
    json!({
        "name": "corp",
        "url": "ldaps://ldap.example.org",
        "bind_dn": SERVICE_DN,
        "bind_password": SERVICE_PASSWORD,
        "user_base_dn": USER_BASE,
    })
}

async fn create(env: &Env) -> Value {
    let reply = env
        .h
        .write(Method::POST, DIRECTORIES, &env.owner, corp())
        .await;
    assert_eq!(reply.status, 201, "{:?}", reply.body);
    reply.body
}

#[tokio::test]
async fn the_owner_creates_a_directory_and_its_password_is_never_returned() {
    each_migrated_backend(async |db: &TestDb| {
        let env = setup(db).await;

        let created = create(&env).await;

        let b = db.backend;
        assert_eq!(created["name"], "corp", "{b}");
        assert_eq!(created["url"], "ldaps://ldap.example.org");
        assert_eq!(created["bind_dn"], SERVICE_DN);
        assert_eq!(created["has_bind_password"], true);
        assert_eq!(created["tls_verify"], true);
        assert_eq!(created["user_filter"], "(objectClass=person)");
        assert_eq!(created["username_attr"], "uid");
        assert_eq!(created["enabled"], true);
        assert_eq!(created["managed_by"], "ui");
        let id = created["id"].as_i64().unwrap();
        let listed = env.h.get(DIRECTORIES, &env.admin).await;
        let one = env.h.get(&path(id), &env.admin).await;
        assert_eq!(listed.status, 200, "{b}: {:?}", listed.body);
        assert_eq!(listed.body[0]["id"], id);
        assert_eq!(one.status, 200);
        for dump in [
            created.to_string(),
            listed.body.to_string(),
            one.body.to_string(),
        ] {
            assert!(!dump.contains(SERVICE_PASSWORD), "{b}: {dump}");
        }
    })
    .await;
}

#[tokio::test]
async fn changing_the_connection_needs_the_password_again() {
    each_migrated_backend(async |db: &TestDb| {
        let env = setup(db).await;
        let id = create(&env).await["id"].as_i64().unwrap();
        let (h, owner) = (&env.h, &env.owner);
        let b = db.backend;

        let moved = json!({ "url": "ldaps://evil.example.net" });
        let moved = h.write(Method::PATCH, &path(id), owner, moved).await;
        let renamed =
            json!({ "name": "Corp LDAP", "group_base_dn": "ou=groups,dc=example,dc=org" });
        let renamed = h.write(Method::PATCH, &path(id), owner, renamed).await;
        let reentered =
            json!({ "url": "ldaps://ldap2.example.org", "bind_password": "new-Pass-1" });
        let reentered = h.write(Method::PATCH, &path(id), owner, reentered).await;

        assert_eq!(moved.status, 422, "{b}: {:?}", moved.body);
        assert!(moved.body["error"]["details"]["fields"]["bind_password"].is_array());
        assert_eq!(renamed.status, 200, "{b}: {:?}", renamed.body);
        assert_eq!(renamed.body["has_bind_password"], true);
        assert_eq!(reentered.status, 200, "{b}: {:?}", reentered.body);
        assert_eq!(reentered.body["url"], "ldaps://ldap2.example.org");
    })
    .await;
}

#[tokio::test]
async fn unsafe_urls_and_a_second_enabled_directory_are_refused() {
    each_migrated_backend(async |db: &TestDb| {
        let env = setup(db).await;
        create(&env).await;
        let (h, owner) = (&env.h, &env.owner);
        let b = db.backend;

        let mut metadata = corp();
        metadata["name"] = json!("metadata");
        metadata["url"] = json!("ldaps://169.254.169.254");
        let metadata = h.write(Method::POST, DIRECTORIES, owner, metadata).await;
        let mut plain = corp();
        plain["name"] = json!("plain");
        plain["url"] = json!("ldap://ldap.example.org");
        let plain = h.write(Method::POST, DIRECTORIES, owner, plain).await;
        let mut second = corp();
        second["name"] = json!("second");
        let second = h.write(Method::POST, DIRECTORIES, owner, second).await;

        for reply in [&metadata, &plain] {
            assert_eq!(reply.status, 422, "{b}: {:?}", reply.body);
            assert!(reply.body["error"]["details"]["fields"]["url"].is_array());
        }
        assert_eq!(second.status, 409, "{b}: {:?}", second.body);
        assert_eq!(second.body["error"]["details"]["field"], "enabled");
    })
    .await;
}

#[tokio::test]
async fn config_managed_directories_are_read_only() {
    each_migrated_backend(async |db: &TestDb| {
        let env = setup(db).await;
        let id = create(&env).await["id"].as_i64().unwrap();
        let mut row: ldap_directory::ActiveModel = ldap_directory::Entity::find_by_id(id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap()
            .into();
        row.managed_by = Set(ManagedBy::Config);
        row.update(&db.conn).await.unwrap();
        let (h, owner) = (&env.h, &env.owner);

        let update = json!({ "enabled": false });
        let update = h.write(Method::PATCH, &path(id), owner, update).await;
        let delete = h.act(Method::DELETE, &path(id), owner).await;

        for reply in [&update, &delete] {
            assert_eq!(reply.status, 409, "{}: {:?}", db.backend, reply.body);
            assert_eq!(reply.code(), "managed_by_config");
        }
        assert_eq!(h.get(&path(id), owner).await.body["managed_by"], "config");
    })
    .await;
}

#[tokio::test]
async fn writes_are_audited_with_the_connection_but_never_the_password() {
    each_migrated_backend(async |db: &TestDb| {
        let env = setup(db).await;
        let id = create(&env).await["id"].as_i64().unwrap();
        let (h, owner) = (&env.h, &env.owner);
        let change = json!({
            "url": "ldap://ldap.example.org",
            "start_tls": true,
            "tls_verify": false,
            "bind_dn": "cn=other,dc=example,dc=org",
            "bind_password": "other-Pass-9",
            "enabled": false,
            "email_attr": "mail",
            "display_name_attr": "cn",
        });
        let changed = h.write(Method::PATCH, &path(id), owner, change).await;
        assert_eq!(changed.status, 200, "{}: {:?}", db.backend, changed.body);
        h.act(Method::DELETE, &path(id), owner).await;

        let rows = h.audit_rows().await;

        let actions: Vec<_> = rows.iter().map(|r| r.action.as_str()).collect();
        let expected = [
            "ldap_directory.create",
            "ldap_directory.update",
            "ldap_directory.delete",
        ];
        assert_eq!(actions, expected, "{}", db.backend);
        assert!(rows.iter().all(|r| r.target_type == "ldap_directory"));
        assert!(rows.iter().all(|r| r.actor_id == Some(owner.user_id)));
        let (before, after) = (snapshot(&rows[1].before), snapshot(&rows[1].after));
        assert_eq!(before["url"], "ldaps://ldap.example.org");
        assert_eq!(after["url"], "ldap://ldap.example.org");
        assert_eq!(
            (before["start_tls"].clone(), after["start_tls"].clone()),
            (json!(false), json!(true))
        );
        assert_eq!(
            (before["tls_verify"].clone(), after["tls_verify"].clone()),
            (json!(true), json!(false))
        );
        assert_eq!(before["bind_dn"], SERVICE_DN);
        assert_eq!(after["bind_dn"], "cn=other,dc=example,dc=org");
        assert!(before["email_attr"].is_null());
        assert_eq!(after["email_attr"], "mail");
        assert_eq!(after["display_name_attr"], "cn");
        assert_eq!(
            (before["enabled"].clone(), after["enabled"].clone()),
            (json!(true), json!(false))
        );
        for row in &rows {
            let dump = format!("{:?} {:?}", row.before, row.after);
            assert!(!dump.contains(SERVICE_PASSWORD), "{dump}");
            assert!(!dump.contains("other-Pass-9"), "{dump}");
        }
    })
    .await;
}

#[tokio::test]
async fn only_the_owner_writes_admins_read_and_users_are_refused() {
    each_migrated_backend(async |db: &TestDb| {
        let env = setup(db).await;
        let id = create(&env).await["id"].as_i64().unwrap();
        let h = &env.h;
        let b = db.backend;
        let patch = json!({ "enabled": false });
        let mut other = corp();
        other["name"] = json!("other");
        other["enabled"] = json!(false);

        for console in [&env.admin, &env.user] {
            let refused = [
                h.write(Method::POST, DIRECTORIES, console, other.clone())
                    .await,
                h.write(Method::PATCH, &path(id), console, patch.clone())
                    .await,
                h.act(Method::DELETE, &path(id), console).await,
            ];
            for reply in &refused {
                assert_eq!(reply.status, 403, "{b}: {:?}", reply.body);
            }
        }
        assert_eq!(h.get(DIRECTORIES, &env.admin).await.status, 200);
        assert_eq!(h.get(&path(id), &env.admin).await.status, 200);
        assert_eq!(h.get(DIRECTORIES, &env.user).await.status, 403);
        assert_eq!(h.get(&path(id), &env.user).await.status, 403);
        let still = h.get(&path(id), &env.owner).await;
        assert_eq!(still.body["enabled"], true, "{b}");
        assert_eq!(
            h.act(Method::DELETE, &path(id), &env.owner).await.status,
            204
        );
        assert_eq!(h.get(&path(id), &env.owner).await.status, 404);
    })
    .await;
}
