//! OIDC provider administration (M3-09): CRUD with the client secret write-only, the fixed
//! callback URL, config-managed providers read-only (409), linked providers not deletable,
//! audit rows without the secret, and the permission matrix (the owner writes, admins read, a user
//! is refused). Runs on SQLite and PostgreSQL.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::Method;
use common::login::harness_with;
use common::tokens::snapshot;
use common::{Console, Harness};
use deskhaven_core::state::StateConfig;
use deskhaven_core::users::Role;
use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::entity::{oidc_provider, user_identity};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, EntityTrait};
use serde_json::{Value, json};

const PROVIDERS: &str = "/admin/api/v1/oidc-providers";
const SECRET: &str = "client-Secret-Value-981";
const ISSUER: &str = "https://idp.example.com";

fn path(id: i64) -> String {
    format!("{PROVIDERS}/{id}")
}

async fn setup(db: &TestDb) -> (Harness, Console) {
    let config = StateConfig {
        public_url: "https://rd.example.com".to_owned(),
        ..StateConfig::default()
    };
    let h = harness_with(db, deskhaven_admin_api::AdminRoutes::new(), &config).await;
    let owner = h.console("own", Role::Owner).await;
    (h, owner)
}

fn corp() -> Value {
    json!({
        "name": "corp",
        "display_name": "Corp SSO",
        "issuer": ISSUER,
        "client_id": "deskhaven",
        "client_secret": SECRET,
        "auto_provision": true,
        "group_claim": "groups",
        "admin_group": "rd-admins",
    })
}

async fn create(h: &Harness, console: &Console, body: Value) -> Value {
    let reply = h.write(Method::POST, PROVIDERS, console, body).await;
    assert_eq!(reply.status, 201, "{:?}", reply.body);
    reply.body
}

async fn make_managed(db: &TestDb, id: i64) {
    let mut row: oidc_provider::ActiveModel = oidc_provider::Entity::find_by_id(id)
        .one(&db.conn)
        .await
        .unwrap()
        .unwrap()
        .into();
    row.managed_by = Set(ManagedBy::Config);
    row.update(&db.conn).await.unwrap();
}

#[tokio::test]
async fn a_provider_is_created_listed_with_the_callback_url_and_never_shows_its_secret() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, owner) = setup(db).await;

        let created = create(&h, &owner, corp()).await;

        let b = db.backend;
        assert_eq!(created["name"], "corp", "{b}");
        assert_eq!(created["display_name"], "Corp SSO");
        assert_eq!(created["issuer"], ISSUER);
        assert_eq!(created["has_client_secret"], true);
        assert_eq!(created["scopes"], "openid profile email");
        assert_eq!(created["admin_group"], "rd-admins");
        assert_eq!(created["enabled"], true);
        assert_eq!(created["managed_by"], "ui");
        let listed = h.get(PROVIDERS, &owner).await;
        assert_eq!(listed.status, 200, "{b}: {:?}", listed.body);
        assert_eq!(
            listed.body["callback_url"],
            "https://rd.example.com/api/oidc/callback"
        );
        assert_eq!(listed.body["items"][0]["id"], created["id"]);
        let one = h.get(&path(created["id"].as_i64().unwrap()), &owner).await;
        assert_eq!(one.status, 200);
        for dump in [
            created.to_string(),
            listed.body.to_string(),
            one.body.to_string(),
        ] {
            assert!(!dump.contains(SECRET), "{b}: {dump}");
        }
    })
    .await;
}

#[tokio::test]
async fn the_secret_is_replaced_or_cleared_only_when_the_patch_names_it() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, owner) = setup(db).await;
        let id = create(&h, &owner, corp()).await["id"].as_i64().unwrap();
        let b = db.backend;

        let renamed = h
            .write(
                Method::PATCH,
                &path(id),
                &owner,
                json!({ "display_name": "SSO" }),
            )
            .await;
        assert_eq!(renamed.status, 200, "{b}: {:?}", renamed.body);
        assert_eq!(renamed.body["has_client_secret"], true);
        assert_eq!(renamed.body["display_name"], "SSO");
        let cleared = h
            .write(
                Method::PATCH,
                &path(id),
                &owner,
                json!({ "client_secret": null, "admin_group": null }),
            )
            .await;
        assert_eq!(cleared.status, 200, "{b}: {:?}", cleared.body);
        assert_eq!(cleared.body["has_client_secret"], false);
        assert!(cleared.body["admin_group"].is_null());
        let set = json!({ "client_secret": "another-Secret-1" });
        let set = h.write(Method::PATCH, &path(id), &owner, set).await;
        assert_eq!(set.body["has_client_secret"], true, "{b}");
    })
    .await;
}

#[tokio::test]
async fn the_issuer_cannot_change_and_bad_input_is_422() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, owner) = setup(db).await;
        let id = create(&h, &owner, corp()).await["id"].as_i64().unwrap();
        let b = db.backend;

        let issuer = json!({ "issuer": "https://evil.example.com" });
        let issuer = h.write(Method::PATCH, &path(id), &owner, issuer).await;
        let mut plain = corp();
        plain["name"] = json!("plain");
        plain["issuer"] = json!("http://idp.example.com");
        let plain = h.write(Method::POST, PROVIDERS, &owner, plain).await;
        let mut taken = corp();
        taken["issuer"] = json!("https://other.example.com");
        let taken = h.write(Method::POST, PROVIDERS, &owner, taken).await;

        assert_eq!(issuer.status, 422, "{b}: {:?}", issuer.body);
        assert_eq!(plain.status, 422, "{b}: {:?}", plain.body);
        assert!(plain.body["error"]["details"]["fields"]["issuer"].is_array());
        assert_eq!(taken.status, 409, "{b}: {:?}", taken.body);
        assert_eq!(taken.code(), "conflict");
    })
    .await;
}

#[tokio::test]
async fn config_managed_providers_are_read_only() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, owner) = setup(db).await;
        let id = create(&h, &owner, corp()).await["id"].as_i64().unwrap();
        make_managed(db, id).await;
        let b = db.backend;

        let update = h
            .write(
                Method::PATCH,
                &path(id),
                &owner,
                json!({ "enabled": false }),
            )
            .await;
        let delete = h.act(Method::DELETE, &path(id), &owner).await;
        let read = h.get(&path(id), &owner).await;

        for reply in [&update, &delete] {
            assert_eq!(reply.status, 409, "{b}: {:?}", reply.body);
            assert_eq!(reply.code(), "managed_by_config");
        }
        assert_eq!(read.body["managed_by"], "config");
        assert_eq!(read.body["enabled"], true);
    })
    .await;
}

#[tokio::test]
async fn deleting_works_and_a_missing_provider_is_404() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, owner) = setup(db).await;
        let id = create(&h, &owner, corp()).await["id"].as_i64().unwrap();

        let deleted = h.act(Method::DELETE, &path(id), &owner).await;
        let again = h.act(Method::DELETE, &path(id), &owner).await;
        let read = h.get(&path(id), &owner).await;

        assert_eq!(deleted.status, 204, "{}: {:?}", db.backend, deleted.body);
        assert_eq!(again.status, 404);
        assert_eq!(read.status, 404);
    })
    .await;
}

#[tokio::test]
async fn a_provider_with_linked_accounts_cannot_be_deleted() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, owner) = setup(db).await;
        let id = create(&h, &owner, corp()).await["id"].as_i64().unwrap();
        let linked = h.create_user("linked", Role::User).await;
        let identity = user_identity::ActiveModel {
            user_id: Set(linked),
            provider_id: Set(id),
            subject: Set("sub-1".to_owned()),
            email: Set(None),
            email_verified: Set(false),
            created_at: Set(deskhaven_db::time::now()),
            ..Default::default()
        };
        identity.insert(&db.conn).await.unwrap();

        let reply = h.act(Method::DELETE, &path(id), &owner).await;

        assert_eq!(reply.status, 409, "{}: {:?}", db.backend, reply.body);
        assert_eq!(reply.code(), "conflict");
        assert_eq!(h.get(&path(id), &owner).await.status, 200);
    })
    .await;
}

#[tokio::test]
async fn writes_are_audited_without_the_secret() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, owner) = setup(db).await;
        let id = create(&h, &owner, corp()).await["id"].as_i64().unwrap();
        let patch = json!({ "client_secret": "rotated-Secret-2", "enabled": false });
        h.write(Method::PATCH, &path(id), &owner, patch).await;
        h.act(Method::DELETE, &path(id), &owner).await;

        let rows = h.audit_rows().await;

        let actions: Vec<_> = rows.iter().map(|r| r.action.as_str()).collect();
        assert_eq!(
            actions,
            [
                "oidc_provider.create",
                "oidc_provider.update",
                "oidc_provider.delete"
            ],
            "{}",
            db.backend
        );
        assert!(rows.iter().all(|r| r.target_type == "oidc_provider"));
        assert!(rows.iter().all(|r| r.target_id == id.to_string()));
        assert!(rows.iter().all(|r| r.actor_id == Some(owner.user_id)));
        let update = &rows[1];
        assert_eq!(snapshot(&update.before)["enabled"], true);
        assert_eq!(snapshot(&update.after)["enabled"], false);
        assert_eq!(snapshot(&update.after)["issuer"], ISSUER);
        for row in &rows {
            let dump = format!("{:?} {:?}", row.before, row.after);
            assert!(
                !dump.contains(SECRET) && !dump.contains("rotated-Secret-2"),
                "{dump}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn only_the_owner_writes_admins_read_and_users_are_refused() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, owner) = setup(db).await;
        let admin = h.console("adm", Role::Admin).await;
        let user = h.console("usr", Role::User).await;
        let id = create(&h, &owner, corp()).await["id"].as_i64().unwrap();
        let b = db.backend;
        let mut second = corp();
        second["name"] = json!("second");
        second["issuer"] = json!("https://second.example.com");
        let patch = json!({ "enabled": false });

        for console in [&admin, &user] {
            let refused = [
                h.write(Method::POST, PROVIDERS, console, second.clone())
                    .await,
                h.write(Method::PATCH, &path(id), console, patch.clone())
                    .await,
                h.act(Method::DELETE, &path(id), console).await,
            ];
            for reply in &refused {
                assert_eq!(reply.status, 403, "{b}: {:?}", reply.body);
            }
        }
        assert_eq!(h.get(PROVIDERS, &admin).await.status, 200);
        assert_eq!(h.get(&path(id), &admin).await.body["enabled"], true, "{b}");
        assert_eq!(h.get(PROVIDERS, &user).await.status, 403);
        assert_eq!(h.get(&path(id), &user).await.status, 403);
        let created = h.write(Method::POST, PROVIDERS, &owner, second).await;
        assert_eq!(created.status, 201, "{b}: {:?}", created.body);
        let patched = h.write(Method::PATCH, &path(id), &owner, patch).await;
        assert_eq!(patched.body["enabled"], false, "{b}");
        assert_eq!(h.act(Method::DELETE, &path(id), &owner).await.status, 204);
    })
    .await;
}

#[tokio::test]
async fn an_empty_secret_in_a_patch_is_refused_and_only_null_clears_it() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, owner) = setup(db).await;
        let id = create(&h, &owner, corp()).await["id"].as_i64().unwrap();

        let empty = json!({ "client_secret": "" });
        let empty = h.write(Method::PATCH, &path(id), &owner, empty).await;

        assert_eq!(empty.status, 422, "{}: {:?}", db.backend, empty.body);
        assert!(empty.body["error"]["details"]["fields"]["client_secret"].is_array());
        assert_eq!(
            h.get(&path(id), &owner).await.body["has_client_secret"],
            true
        );
    })
    .await;
}

#[tokio::test]
async fn audit_snapshots_carry_the_icon() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, owner) = setup(db).await;
        let id = create(&h, &owner, corp()).await["id"].as_i64().unwrap();
        let icon = json!({ "icon": "<svg/>" });
        let patched = h.write(Method::PATCH, &path(id), &owner, icon).await;
        assert_eq!(patched.status, 200, "{}: {:?}", db.backend, patched.body);

        let rows = h.audit_rows().await;

        let update = rows
            .iter()
            .find(|r| r.action == "oidc_provider.update")
            .unwrap();
        assert!(snapshot(&update.before)["icon"].is_null());
        assert_eq!(snapshot(&update.after)["icon"], "<svg/>");
    })
    .await;
}
