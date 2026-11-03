//! `GET /client-config` (M3-11): the server addresses, public key, config string, QR code
//! and Windows file names for RustDesk clients. Admins and owners only. Runs on SQLite and
//! PostgreSQL.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::{Method, StatusCode};
use common::login::harness_with;
use common::{Harness, auth_headers, harness, request};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::client_config::ServerConfig;
use deskhaven_core::state::StateConfig;
use deskhaven_core::users::Role;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::{Value, json};

const PATH: &str = "/admin/api/v1/client-config";
const SIGNAL_KEY: &str = "test-signal-key";

async fn with_public_url(db: &TestDb, public_url: &str) -> Harness {
    let config = StateConfig {
        public_url: public_url.to_owned(),
        console_url: String::new(),
    };
    harness_with(db, AdminRoutes::new(), &config).await
}

#[tokio::test]
async fn an_admin_gets_the_addresses_key_and_config_string() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let h = with_public_url(db, "https://rd.example.com/").await;
        let admin = h.console("adm", Role::Admin).await;
        h.set_setting("signal.relay_servers", json!("relay.example.com:21117"))
            .await;

        // Act
        let reply = h.get(PATH, &admin).await;

        // Assert
        assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
        let body = &reply.body;
        assert_eq!(body["id_server"], "rd.example.com");
        assert_eq!(body["relay_server"], "relay.example.com");
        assert_eq!(body["api_server"], "https://rd.example.com");
        assert_eq!(body["public_key"], SIGNAL_KEY);
        assert_eq!(body["host_source"], "public_url");
        assert_eq!(body["relay_servers"], json!(["relay.example.com:21117"]));
        let expected = ServerConfig {
            host: "rd.example.com".to_owned(),
            relay: "relay.example.com".to_owned(),
            api: "https://rd.example.com".to_owned(),
            key: SIGNAL_KEY.to_owned(),
        };
        assert_eq!(body["config_string"], expected.config_string().unwrap());
        assert_eq!(
            body["windows_filename_encoded"],
            expected.windows_names().encoded.unwrap()
        );
        assert_eq!(body["windows_filename_plain"], Value::Null);
        assert_eq!(body["qr"]["size"].as_i64().unwrap() > 0, true);
        assert!(body["qr"]["path"].as_str().unwrap().starts_with('M'));
    })
    .await;
}

#[tokio::test]
async fn without_a_public_url_the_console_host_is_used_and_flagged() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let owner = h.console("own", Role::Owner).await;

        let reply = h
            .raw(request(
                Method::GET,
                PATH,
                &auth_headers(&owner, &[("host", "Console.Test:8443")]),
                None,
            ))
            .await;

        assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
        assert_eq!(reply.body["id_server"], "console.test");
        assert_eq!(reply.body["api_server"], "");
        assert_eq!(reply.body["host_source"], "request_host");
        assert!(reply.body["config_string"].is_string());
        // No API URL and a key without a slash: the readable name works too.
        assert_eq!(
            reply.body["windows_filename_plain"],
            "rustdesk-host=console.test,key=test-signal-key,.exe"
        );
        assert!(reply.body["windows_filename_encoded"].is_string());
    })
    .await;
}

#[tokio::test]
async fn without_any_usable_host_there_is_no_config_string() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let admin = h.console("adm", Role::Admin).await;

        let reply = h.get(PATH, &admin).await;

        assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
        assert_eq!(reply.body["host_source"], "unknown");
        assert_eq!(reply.body["id_server"], "");
        assert_eq!(reply.body["public_key"], SIGNAL_KEY);
        for field in ["config_string", "qr", "windows_filename_encoded"] {
            assert_eq!(reply.body[field], Value::Null, "{field}");
        }
    })
    .await;
}

#[tokio::test]
async fn plain_users_and_strangers_are_refused() {
    each_migrated_backend(async |db: &TestDb| {
        let h = with_public_url(db, "https://rd.example.com").await;
        let user = h.console("plain", Role::User).await;

        let as_user = h.get(PATH, &user).await;
        let anonymous = h.send(Method::GET, PATH, None, None, None).await;

        assert_eq!(as_user.status, StatusCode::FORBIDDEN);
        assert_eq!(as_user.code(), "forbidden");
        assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
        assert!(!as_user.body.to_string().contains(SIGNAL_KEY));
    })
    .await;
}
