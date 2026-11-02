//! Outbound HTTP hardening of the OIDC client: response size cap, no redirects, request
//! timeout, and endpoints from the discovery document checked before use (no `javascript:`
//! authorization URL, no plain http outside loopback). Service-level cases on both backends.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::time::Duration;

use common::oidc::{Env, device};
use common::oidc_idp::ISSUER_PATH;
use deskhaven_core::oidc::{FetchError, HttpFetch, MAX_RESPONSE_BYTES, OidcError, ReqwestFetch};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use openidconnect::http;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn get(url: &str) -> openidconnect::HttpRequest {
    http::Request::builder()
        .method("GET")
        .uri(url)
        .body(Vec::new())
        .unwrap()
}

#[tokio::test]
async fn responses_over_the_size_cap_are_refused() {
    let server = MockServer::start().await;
    Mock::given(path("/big"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'x'; MAX_RESPONSE_BYTES + 1]))
        .mount(&server)
        .await;
    Mock::given(path("/fits"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'x'; MAX_RESPONSE_BYTES]))
        .mount(&server)
        .await;
    let fetch = ReqwestFetch::new();

    let big = fetch.fetch(get(&format!("{}/big", server.uri()))).await;
    let fits = fetch.fetch(get(&format!("{}/fits", server.uri()))).await;

    assert!(matches!(big, Err(FetchError::TooLarge)), "{big:?}");
    assert_eq!(fits.unwrap().body().len(), MAX_RESPONSE_BYTES);
}

#[tokio::test]
async fn redirects_are_returned_not_followed() {
    let server = MockServer::start().await;
    let target = format!("{}/elsewhere", server.uri());
    Mock::given(path("/moved"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", target.as_str()))
        .mount(&server)
        .await;
    Mock::given(path("/elsewhere"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let reply = ReqwestFetch::new()
        .fetch(get(&format!("{}/moved", server.uri())))
        .await
        .unwrap();

    assert_eq!(reply.status(), 302);
}

#[tokio::test]
async fn slow_responses_time_out() {
    let server = MockServer::start().await;
    Mock::given(path("/slow"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(3)))
        .mount(&server)
        .await;
    let fetch = ReqwestFetch::with_timeouts(Duration::from_secs(1), Duration::from_millis(300));

    let started = std::time::Instant::now();
    let reply = fetch.fetch(get(&format!("{}/slow", server.uri()))).await;

    assert!(matches!(reply, Err(FetchError::Transport(_))), "{reply:?}");
    assert!(started.elapsed() < Duration::from_secs(2));
}

/// The provider's discovery document replaced by `document(issuer)`.
async fn with_discovery(env: &Env, document: serde_json::Value) {
    env.idp.server.reset().await;
    env.idp.mount_discovery(document).await;
}

#[tokio::test]
async fn unusable_discovered_endpoints_make_the_provider_unavailable() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let base = env.idp.discovery();
        let with = |key: &str, value: &str| {
            let mut doc = base.clone();
            doc[key] = json!(value);
            doc
        };
        let cases = [
            (
                "javascript authorization endpoint",
                with("authorization_endpoint", "javascript:alert(1)"),
            ),
            (
                "plain http token endpoint off loopback",
                with("token_endpoint", "http://192.0.2.1/token"),
            ),
            (
                "plain http JWKS off loopback",
                with("jwks_uri", "http://192.0.2.1/certs"),
            ),
            (
                "credentials in the token endpoint",
                with("token_endpoint", "https://u:p@idp.example/token"),
            ),
        ];
        for (case, document) in cases {
            with_discovery(&env, document).await;
            let result = env.oidc.begin_client("corp", &device()).await;
            assert!(
                matches!(result, Err(OidcError::Unavailable(_))),
                "{}: {case}: {result:?}",
                db.backend
            );
        }
    })
    .await;
}

#[tokio::test]
async fn a_redirected_discovery_document_is_not_followed() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        env.idp.server.reset().await;
        Mock::given(method("GET"))
            .and(path(format!(
                "{ISSUER_PATH}/.well-known/openid-configuration"
            )))
            .respond_with(
                ResponseTemplate::new(302).insert_header("location", "https://evil.example.com/"),
            )
            .mount(&env.idp.server)
            .await;

        let result = env.oidc.begin_client("corp", &device()).await;

        assert!(
            matches!(result, Err(OidcError::Unavailable(_))),
            "{}: {result:?}",
            db.backend
        );
    })
    .await;
}
