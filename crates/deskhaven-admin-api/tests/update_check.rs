//! `GET /update-check`: admins only; empty while the check is off or nothing newer is known;
//! the newest release and a link built from its version once the server has seen one. GitHub
//! is never contacted: the harness answers every outbound request.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use axum::http::Method;
use common::{FixedRelease, Harness, harness, harness_with_updates};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_config::bootstrap::{LoadOptions, load};
use deskhaven_config::settings::PinSet;
use deskhaven_core::build::SOURCE_URL;
use deskhaven_core::users::Role;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::{Value, json};

const UPDATE_CHECK: &str = "/admin/api/v1/update-check";
const NOTHING: fn() -> Value = || json!({ "latest": null, "release_url": null });

#[tokio::test]
async fn only_admins_ask_for_updates() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let user = h.console("bob", Role::User).await;
        let admin = h.console("adm", Role::Admin).await;
        let owner = h.console("own", Role::Owner).await;

        let anonymous = h.send(Method::GET, UPDATE_CHECK, None, None, None).await;
        assert_eq!(anonymous.status, 401);
        let denied = h.get(UPDATE_CHECK, &user).await;
        assert_eq!((denied.status.as_u16(), denied.code()), (403, "forbidden"));
        for console in [&admin, &owner] {
            let reply = h.get(UPDATE_CHECK, console).await;
            assert_eq!(reply.status, 200, "{}", reply.body);
            assert_eq!(reply.body, NOTHING());
        }
    })
    .await;
}

#[tokio::test]
async fn a_pinned_off_check_answers_nothing_and_asks_nobody() {
    each_migrated_backend(async |db: &TestDb| {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deskhaven.toml");
        std::fs::write(&path, "[settings]\n\"updates.check\" = false\n").unwrap();
        let options = LoadOptions {
            config_path: Some(path),
            search_paths: Vec::new(),
            cli: Default::default(),
        };
        let pins = PinSet::from_bootstrap(&load(&options).unwrap().config.settings).unwrap();
        let fetch = Arc::new(FixedRelease::new("v99.0.0"));
        let h = harness_with_updates(db, pins, Arc::clone(&fetch) as _).await;
        let admin = h.console("adm", Role::Admin).await;

        for _ in 0..2 {
            let reply = h.get(UPDATE_CHECK, &admin).await;
            assert_eq!((reply.status.as_u16(), reply.body), (200, NOTHING()));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(fetch.calls(), 0);
    })
    .await;
}

/// The first request starts the check and answers from the empty cache; a later one carries
/// the release.
#[tokio::test]
async fn a_newer_release_comes_with_its_release_page() {
    each_migrated_backend(async |db: &TestDb| {
        let fetch = Arc::new(FixedRelease::new("v99.0.0"));
        let h = harness_with_updates(db, PinSet::empty(), Arc::clone(&fetch) as _).await;
        let admin = h.console("adm", Role::Admin).await;

        assert_eq!(h.get(UPDATE_CHECK, &admin).await.body, NOTHING());
        let body = until_offered(&h, &admin).await;

        let expected = json!({
            "latest": "99.0.0",
            "release_url": format!("{SOURCE_URL}/releases/tag/v99.0.0"),
        });
        assert_eq!(body, expected);
        assert_eq!(fetch.calls(), 1);
    })
    .await;
}

async fn until_offered(h: &Harness, admin: &common::Console) -> Value {
    for _ in 0..500 {
        let reply = h.get(UPDATE_CHECK, admin).await;
        assert_eq!(reply.status, 200, "{}", reply.body);
        if !reply.body["latest"].is_null() {
            return reply.body;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no release offered");
}
