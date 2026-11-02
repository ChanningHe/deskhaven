//! `BearerUser` through the production stack (03 §1.2-1.3): a test-only route behind the
//! extractor, a session issued by the real services (`client-session` seed), and the
//! 401 `{"error"}` contract for every bad credential. No real authenticated endpoint
//! exists yet (M2-07), hence the test route and the temporary contract directory.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::Path;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::routing::post;
use deskhaven_client_api::{BearerUser, UNAUTHORIZED_MESSAGE};
use deskhaven_contract::app::TestApp;
use deskhaven_contract::fixture::Request;
use deskhaven_contract::mirror::ErrorBody;
use deskhaven_contract::runner::{Setup, run_dir_with};
use deskhaven_contract::seeds::{SEED_PASSWORD, SEED_RUSTDESK_ID, SEED_UUID, Seeds};
use deskhaven_core::state::AppState;
use deskhaven_core::users::{Credential, NewUser, Password};
use serde_json::{Value, json};

const COMMIT_FILE: &str = "UPSTREAM_COMMIT";
const WHOAMI: &str = "/test/whoami";

async fn whoami(BearerUser(principal): BearerUser) -> Json<Value> {
    let device = principal.device.unwrap();
    Json(json!({
        "user_id": principal.user_id,
        "rustdesk_id": device.rustdesk_id,
        "uuid": device.uuid,
    }))
}

fn routes() -> Router<Arc<AppState>> {
    Router::new().route(WHOAMI, post(whoami))
}

fn upstream_commit() -> String {
    let real = deskhaven_contract::runner::contract_dir().join(COMMIT_FILE);
    std::fs::read_to_string(real).unwrap().trim().to_owned()
}

fn write_fixture(dir: &Path, name: &str, fixture: &Value) {
    let path = dir.join("fixtures").join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_string_pretty(fixture).unwrap()).unwrap();
}

fn fixture(commit: &str, seed: &[&str], headers: Value, expect: Value) -> Value {
    json!({
        "upstream_commit": commit,
        "source": ["flutter/lib/common.dart:2729", "flutter/lib/models/user_model.dart:85-88"],
        "description": "bearer extractor on a test-only route",
        "seed": seed,
        "request": { "method": "POST", "path": WHOAMI, "headers": headers,
                     "body": { "json": { "id": SEED_RUSTDESK_ID, "uuid": SEED_UUID } } },
        "expect": expect
    })
}

#[tokio::test]
async fn seeded_session_token_authenticates_and_bad_ones_get_the_401_contract() {
    let dir = tempfile::tempdir().unwrap();
    let commit = upstream_commit();
    std::fs::write(dir.path().join(COMMIT_FILE), format!("{commit}\n")).unwrap();
    let unauthorized = json!({ "status": 401, "body_kind": "json", "shape": "ErrorBody",
                               "json_subset": { "error": UNAUTHORIZED_MESSAGE } });
    let fixtures = [
        (
            "bearer/valid.json",
            fixture(
                &commit,
                &["client-session"],
                json!({ "authorization": "Bearer ${session_token}" }),
                json!({ "status": 200, "body_kind": "json",
                        "json_subset": { "rustdesk_id": SEED_RUSTDESK_ID, "uuid": SEED_UUID } }),
            ),
        ),
        (
            "bearer/missing.json",
            fixture(
                &commit,
                &["client-session"],
                json!({}),
                unauthorized.clone(),
            ),
        ),
        (
            "bearer/unknown.json",
            fixture(
                &commit,
                &["client-session"],
                json!({ "authorization": format!("Bearer {}", "x".repeat(43)) }),
                unauthorized,
            ),
        ),
    ];
    for (name, fixture) in &fixtures {
        write_fixture(dir.path(), name, fixture);
    }
    let setup = Setup {
        seeds: Seeds::builtin(),
        routes: routes(),
    };

    let outcome = run_dir_with(dir.path(), None, &setup).await.unwrap();

    assert!(outcome.failures.is_empty(), "{outcome}");
    assert_eq!(outcome.passed.len(), fixtures.len());
}

async fn status_and_error(app: &TestApp, authorization: Option<String>) -> (u16, Option<String>) {
    let mut request = Request::new("POST", WHOAMI);
    if let Some(value) = authorization {
        request.headers.insert("authorization".to_owned(), value);
    }
    let reply = app.send(&request).await.unwrap();
    let error = reply.json::<ErrorBody>().ok().map(|e| e.error);
    (reply.status, error)
}

#[tokio::test]
async fn header_variants_follow_the_documented_parsing_rules() {
    let app = TestApp::start_with(routes()).await.unwrap();
    let outputs = Seeds::builtin()
        .apply(&["client-session".to_owned()], &app.state)
        .await
        .unwrap();
    let token = &outputs["session_token"];
    let rejected = (401, Some(UNAUTHORIZED_MESSAGE.to_owned()));

    for accepted in [
        format!("Bearer {token}"),
        format!("bearer {token}"),
        format!("BEARER  {token}"),
    ] {
        assert_eq!(
            status_and_error(&app, Some(accepted.clone())).await.0,
            200,
            "{accepted:?}"
        );
    }
    for bad in [
        None,
        Some(String::new()),
        Some("Bearer".to_owned()),
        Some(token.clone()),
        Some(format!("Basic {token}")),
        Some(format!("Bearer {token} extra")),
        Some(format!("Bearer{token}")),
        Some(format!("Bearer {}", &token[1..])),
    ] {
        assert_eq!(
            status_and_error(&app, bad.clone()).await,
            rejected,
            "{bad:?}"
        );
    }
}

#[tokio::test]
async fn logout_through_the_service_turns_the_next_request_into_a_401() {
    let app = TestApp::start_with(routes()).await.unwrap();
    let outputs = Seeds::builtin()
        .apply(&["client-session".to_owned()], &app.state)
        .await
        .unwrap();
    let bearer = format!("Bearer {}", outputs["session_token"]);
    assert_eq!(status_and_error(&app, Some(bearer.clone())).await.0, 200);

    let revoked = app
        .state
        .sessions
        .revoke_token(&outputs["session_token"])
        .await
        .unwrap();

    assert!(revoked);
    assert_eq!(
        status_and_error(&app, Some(bearer)).await,
        (401, Some(UNAUTHORIZED_MESSAGE.to_owned()))
    );
}

#[tokio::test]
async fn a_console_session_token_is_no_client_credential() {
    let app = TestApp::start_with(routes()).await.unwrap();
    let password = Password::new(SEED_PASSWORD);
    let users = &app.state.users;
    users
        .create(NewUser::new("carol", Credential::Local { password }))
        .await
        .unwrap();
    let login = users
        .authenticate_local("carol", &Password::new(SEED_PASSWORD))
        .await
        .unwrap();
    let (_, console) = app
        .state
        .sessions
        .create_console(login.user.id, &login.stamp, None, None)
        .await
        .unwrap();

    let reply = status_and_error(&app, Some(format!("Bearer {}", console.expose()))).await;

    assert_eq!(reply, (401, Some(UNAUTHORIZED_MESSAGE.to_owned())));
}
