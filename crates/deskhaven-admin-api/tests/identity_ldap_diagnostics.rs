//! LDAP "test connection" / "test user" (M3-09): structured per-step diagnostics for a
//! saved directory or an unsaved form, owner-only, rate limited per client, the same URL
//! filter as logins, and `test_user` with a password counted against the tested
//! username's lockout like a login (it is a password oracle). Runs on SQLite and
//! PostgreSQL against the fake directory.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

#[path = "../../deskhaven-core/tests/common/ldap_fake.rs"]
mod ldap_fake;

use std::time::Duration;

use axum::http::Method;
use common::login::harness_with_ldap;
use common::{Console, Harness, Reply};
use deskhaven_core::groups::NewGroup;
use deskhaven_core::ldap::NewDirectory;
use deskhaven_core::users::Role;
use deskhaven_db::entity::console_audit;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use ldap_fake::{
    FakeDirectory, GROUP_BASE, SERVICE_DN, SERVICE_PASSWORD, USER_BASE, USER_PASSWORD, group_dn,
};
use serde_json::{Value, json};

const CONNECTION: &str = "/admin/api/v1/ldap-tests/connection";
const USER: &str = "/admin/api/v1/ldap-tests/user";

struct Env {
    h: Harness,
    fake: FakeDirectory,
    owner: Console,
    directory_id: i64,
}

async fn setup(db: &TestDb) -> Env {
    let fake = FakeDirectory::new();
    let alice = fake.add_user("alice");
    fake.set_members(&group_dn("staff"), &[&alice]);
    fake.set_members(&group_dn("admins"), &[&alice]);
    let h = harness_with_ldap(db, fake.connector()).await;
    let mut new = NewDirectory::new("corp", "ldaps://ldap.example.org", USER_BASE);
    new.bind_dn = Some(SERVICE_DN.to_owned());
    new.bind_password = Some(SERVICE_PASSWORD.to_owned());
    new.group_base_dn = Some(GROUP_BASE.to_owned());
    new.admin_group_dn = Some(group_dn("admins"));
    let directory_id = h.state.ldap.create_directory(new).await.unwrap().id;
    h.state.groups.create(NewGroup::new("Staff")).await.unwrap();
    let owner = h.console("own", Role::Owner).await;
    Env {
        h,
        fake,
        owner,
        directory_id,
    }
}

fn form(url: &str) -> Value {
    json!({
        "name": "draft",
        "url": url,
        "bind_dn": SERVICE_DN,
        "bind_password": SERVICE_PASSWORD,
        "user_base_dn": USER_BASE,
    })
}

async fn test_user(env: &Env, username: &str, password: Option<&str>) -> Reply {
    let body =
        json!({ "directory_id": env.directory_id, "username": username, "password": password });
    env.h.write(Method::POST, USER, &env.owner, body).await
}

/// The audit row of `action`: tests are recorded by the gate's detached fallback writer,
/// which may still be running when the response arrives.
async fn audit_row(h: &Harness, action: &str) -> console_audit::Model {
    for _ in 0..200 {
        if let Some(row) = h
            .audit_rows()
            .await
            .into_iter()
            .find(|r| r.action == action)
        {
            return row;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no {action} audit row");
}

/// `(step, status)` of every step.
fn steps(reply: &Reply) -> Vec<(String, String)> {
    let steps = reply.body["steps"].as_array().unwrap();
    steps
        .iter()
        .map(|s| {
            let field = |k: &str| s[k].as_str().unwrap().to_owned();
            (field("step"), field("status"))
        })
        .collect()
}

fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
    expected
        .iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect()
}

#[tokio::test]
async fn test_connection_reports_each_step_for_a_saved_directory_and_a_form() {
    each_migrated_backend(async |db: &TestDb| {
        let env = setup(db).await;
        let (h, owner) = (&env.h, &env.owner);
        let b = db.backend;

        let saved = json!({ "directory_id": env.directory_id });
        let saved = h.write(Method::POST, CONNECTION, owner, saved).await;
        let unsaved = json!({ "directory": form("ldaps://other.example.org") });
        let unsaved = h.write(Method::POST, CONNECTION, owner, unsaved).await;
        env.fake.set_down(true);
        let down = json!({ "directory_id": env.directory_id });
        let down = h.write(Method::POST, CONNECTION, owner, down).await;

        assert_eq!(saved.status, 200, "{b}: {:?}", saved.body);
        assert_eq!(saved.body["passed"], true);
        let ok = pairs(&[("connect", "passed"), ("service_bind", "passed")]);
        assert_eq!(steps(&saved), ok);
        assert!(
            saved.body["steps"][1]["message"]
                .as_str()
                .unwrap()
                .contains(SERVICE_DN)
        );
        assert_eq!(unsaved.status, 200, "{b}: {:?}", unsaved.body);
        assert_eq!(steps(&unsaved), ok);
        assert_eq!(down.body["passed"], false);
        let failed = pairs(&[("connect", "failed"), ("service_bind", "skipped")]);
        assert_eq!(steps(&down), failed);
        for reply in [&saved, &unsaved, &down] {
            assert!(!reply.body.to_string().contains(SERVICE_PASSWORD), "{b}");
        }
    })
    .await;
}

#[tokio::test]
async fn test_user_reports_search_bind_and_group_mapping() {
    each_migrated_backend(async |db: &TestDb| {
        let env = setup(db).await;
        let b = db.backend;

        let right = test_user(&env, "alice", Some(USER_PASSWORD)).await;
        let wrong = test_user(&env, "alice", Some("not-it")).await;
        let blank = test_user(&env, "alice", None).await;
        let missing = test_user(&env, "nobody", None).await;

        assert_eq!(right.status, 200, "{b}: {:?}", right.body);
        assert_eq!(right.body["passed"], true);
        let all = pairs(&[
            ("connect", "passed"),
            ("service_bind", "passed"),
            ("user_search", "passed"),
            ("user_bind", "passed"),
            ("group_mapping", "passed"),
        ]);
        assert_eq!(steps(&right), all);
        let user = &right.body["user"];
        assert_eq!(user["dn"], "uid=alice,ou=people,dc=example,dc=org");
        assert_eq!(user["username"], "alice");
        assert_eq!(user["mapped_groups"], json!(["Staff"]));
        assert_eq!(user["admin"], true);
        assert_eq!(wrong.body["passed"], false, "{b}: {:?}", wrong.body);
        assert_eq!(
            steps(&wrong)[3],
            ("user_bind".to_owned(), "failed".to_owned())
        );
        assert_eq!(
            steps(&wrong)[4],
            ("group_mapping".to_owned(), "skipped".to_owned())
        );
        assert_eq!(
            steps(&blank)[3],
            ("user_bind".to_owned(), "skipped".to_owned())
        );
        assert_eq!(
            steps(&missing)[2],
            ("user_search".to_owned(), "failed".to_owned())
        );
        assert!(missing.body["user"].is_null());
        for reply in [&right, &wrong] {
            assert!(!reply.body.to_string().contains(USER_PASSWORD), "{b}");
        }
    })
    .await;
}

#[tokio::test]
async fn failed_test_user_passwords_count_against_the_lockout_like_logins() {
    each_migrated_backend(async |db: &TestDb| {
        let env = setup(db).await;
        env.h
            .set_setting("auth.lockout.max_failures", json!(2))
            .await;
        let b = db.backend;

        let first = test_user(&env, "alice", Some("wrong-1")).await;
        // A right password is not a failure, and no password is no attempt.
        let right = test_user(&env, "alice", Some(USER_PASSWORD)).await;
        let blank = test_user(&env, "alice", None).await;
        let second = test_user(&env, "alice", Some("wrong-2")).await;
        let locked = test_user(&env, "alice", Some(USER_PASSWORD)).await;
        let login = env.h.login("alice", USER_PASSWORD).await;
        let other = test_user(&env, "bob", Some("wrong-3")).await;

        for reply in [&first, &right, &blank, &second, &other] {
            assert_eq!(reply.status, 200, "{b}: {:?}", reply.body);
        }
        assert_eq!(right.body["passed"], true);
        assert_eq!(locked.status, 429, "{b}: {:?}", locked.body);
        assert_eq!(locked.code(), "rate_limited");
        assert!(locked.headers.contains_key("retry-after"));
        assert_eq!(login.status, 429, "{b}: {:?}", login.body);
    })
    .await;
}

#[tokio::test]
async fn diagnostics_are_rate_limited_per_client() {
    each_migrated_backend(async |db: &TestDb| {
        let env = setup(db).await;
        env.h
            .set_setting("auth.ip_rate_limit.per_minute", json!(2))
            .await;
        let saved = json!({ "directory_id": env.directory_id });

        let one = env
            .h
            .write(Method::POST, CONNECTION, &env.owner, saved.clone())
            .await;
        let two = test_user(&env, "alice", None).await;
        let three = env
            .h
            .write(Method::POST, CONNECTION, &env.owner, saved)
            .await;

        assert_eq!(one.status, 200, "{}: {:?}", db.backend, one.body);
        assert_eq!(two.status, 200, "{:?}", two.body);
        assert_eq!(three.status, 429, "{:?}", three.body);
        assert_eq!(three.code(), "rate_limited");
    })
    .await;
}

#[tokio::test]
async fn forms_go_through_the_same_url_filter_and_targets_must_be_exactly_one() {
    each_migrated_backend(async |db: &TestDb| {
        let env = setup(db).await;
        let (h, owner) = (&env.h, &env.owner);
        let b = db.backend;
        env.fake.clear_log();

        let metadata = json!({ "directory": form("ldaps://169.254.169.254") });
        let metadata = h.write(Method::POST, CONNECTION, owner, metadata).await;
        let spelled = json!({ "directory": form("ldaps://0xa9fea9fe"), "username": "alice" });
        let spelled = h.write(Method::POST, USER, owner, spelled).await;
        let both =
            json!({ "directory_id": env.directory_id, "directory": form("ldaps://x.example.org") });
        let both = h.write(Method::POST, CONNECTION, owner, both).await;
        let neither = h.write(Method::POST, CONNECTION, owner, json!({})).await;
        let unknown = h
            .write(
                Method::POST,
                CONNECTION,
                owner,
                json!({ "directory_id": 9999 }),
            )
            .await;

        for reply in [&metadata, &spelled] {
            assert_eq!(reply.status, 422, "{b}: {:?}", reply.body);
            assert!(reply.body["error"]["details"]["fields"]["url"].is_array());
        }
        for reply in [&both, &neither] {
            assert_eq!(reply.status, 422, "{b}: {:?}", reply.body);
        }
        assert_eq!(unknown.status, 404, "{b}: {:?}", unknown.body);
        assert_eq!(env.fake.connects(), 0, "{b}: nothing was contacted");
    })
    .await;
}

#[tokio::test]
async fn diagnostics_are_owner_only_and_audited_without_the_password() {
    each_migrated_backend(async |db: &TestDb| {
        let env = setup(db).await;
        let h = &env.h;
        let admin = h.console("adm", Role::Admin).await;
        let user = h.console("usr", Role::User).await;
        let b = db.backend;
        let connection = json!({ "directory_id": env.directory_id });
        let probe = json!({ "directory_id": env.directory_id, "username": "alice", "password": USER_PASSWORD });

        for console in [&admin, &user] {
            for (path, body) in [(CONNECTION, &connection), (USER, &probe)] {
                let reply = h.write(Method::POST, path, console, body.clone()).await;
                assert_eq!(reply.status, 403, "{b}: {path} {:?}", reply.body);
            }
        }
        assert_eq!(env.fake.connects(), 0, "{b}");
        let tested = h.write(Method::POST, USER, &env.owner, probe).await;
        assert_eq!(tested.status, 200, "{b}: {:?}", tested.body);

        let row = audit_row(h, "ldap_directory.test_user").await;
        assert_eq!(row.actor_id, Some(env.owner.user_id));
        assert_eq!(row.target_id, env.directory_id.to_string());
        let dump = format!("{:?} {:?}", row.before, row.after);
        assert!(dump.contains("alice"), "{dump}");
        assert!(!dump.contains(USER_PASSWORD), "{dump}");
    })
    .await;
}
