//! The 2FA enforcement policy of the identity page (M3-09) is the `auth.tfa.enforce`
//! setting behind `/settings`: the owner changes it with its version, admins and users are
//! refused, a value pinned by the bootstrap file is read-only. Runs on SQLite and
//! PostgreSQL.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::Method;
use common::{harness, harness_pinned};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_config::bootstrap::{LoadOptions, load};
use deskhaven_config::settings::PinSet;
use deskhaven_core::users::Role;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::{Value, json};

const SETTINGS: &str = "/admin/api/v1/settings";
const KEY: &str = "auth.tfa.enforce";

fn enforce(list: &Value) -> &Value {
    let all = list.as_array().unwrap();
    all.iter().find(|s| s["key"] == KEY).unwrap()
}

fn change(version: &Value, value: &str) -> Value {
    json!({ "changes": [{ "key": KEY, "version": version, "value": value }] })
}

#[tokio::test]
async fn the_owner_sets_the_policy_and_admins_and_users_are_refused() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let owner = h.console("own", Role::Owner).await;
        let admin = h.console("adm", Role::Admin).await;
        let user = h.console("usr", Role::User).await;
        let b = db.backend;
        let listed = h.get(SETTINGS, &owner).await;
        let current = enforce(&listed.body);
        assert_eq!(current["value"], "off", "{b}");
        assert_eq!(
            current["schema"]["variants"],
            json!(["off", "admins", "all"])
        );

        let refused = [
            h.get(SETTINGS, &admin).await,
            h.write(
                Method::PATCH,
                SETTINGS,
                &admin,
                change(&current["version"], "all"),
            )
            .await,
            h.write(
                Method::PATCH,
                SETTINGS,
                &user,
                change(&current["version"], "all"),
            )
            .await,
        ];
        let changed = h
            .write(
                Method::PATCH,
                SETTINGS,
                &owner,
                change(&current["version"], "admins"),
            )
            .await;

        for reply in &refused {
            assert_eq!(reply.status, 403, "{b}: {:?}", reply.body);
        }
        assert_eq!(changed.status, 200, "{b}: {:?}", changed.body);
        assert_eq!(enforce(&changed.body)["value"], "admins");
        assert_eq!(enforce(&changed.body)["source"]["kind"], "database");
    })
    .await;
}

#[tokio::test]
async fn a_pinned_policy_is_read_only() {
    each_migrated_backend(async |db: &TestDb| {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deskhaven.toml");
        std::fs::write(&path, "[settings]\n\"auth.tfa.enforce\" = \"off\"\n").unwrap();
        let options = LoadOptions {
            config_path: Some(path),
            search_paths: Vec::new(),
            cli: Default::default(),
        };
        let pins = PinSet::from_bootstrap(&load(&options).unwrap().config.settings).unwrap();
        let h = harness_pinned(db, AdminRoutes::new(), pins).await;
        let owner = h.console("own", Role::Owner).await;

        let listed = h.get(SETTINGS, &owner).await;
        let current = enforce(&listed.body);
        let reply = h
            .write(
                Method::PATCH,
                SETTINGS,
                &owner,
                change(&current["version"], "all"),
            )
            .await;

        assert_eq!(current["pinned"], true, "{}", db.backend);
        assert_eq!(current["value"], "off");
        assert_eq!(reply.status, 409, "{:?}", reply.body);
        assert_eq!(reply.code(), "setting_pinned");
    })
    .await;
}
