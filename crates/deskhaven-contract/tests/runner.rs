//! The runner must fail loudly when a fixture is wrong: these fixtures live
//! in a temporary contract directory and are expected to fail. A test-only
//! echo route and test-only seeds prove what reaches the wire.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::Path;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, Uri};
use axum::routing::any;
use deskhaven_config::settings::Change;
use deskhaven_contract::runner::{Outcome, Setup, run_dir_with};
use deskhaven_contract::seeds::{SeedOutputs, Seeds};
use deskhaven_core::settings::UpdateRequest;
use deskhaven_core::state::AppState;
use serde_json::{Map, Value, json};

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const DEMO_PORT: u16 = 2525;

fn write_fixture(dir: &Path, name: &str, fixture: &Value) {
    let path = dir.join("fixtures").join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, fixture.to_string()).unwrap();
}

fn fixture(request: Value, expect: Value) -> Value {
    json!({
        "upstream_commit": COMMIT,
        "source": ["src/hbbs_http.rs:24-39"],
        "description": "runner self-test",
        "request": request,
        "expect": expect
    })
}

fn seeded(seed: &[&str], request: Value, expect: Value) -> Value {
    let mut fixture = fixture(request, expect);
    fixture["seed"] = json!(seed);
    fixture
}

/// Echoes what the handler received, plus `smtp.port` as the shared state
/// holds it (written by the `smtp-port` seed).
async fn echo(
    State(state): State<Arc<AppState>>,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> Json<Value> {
    let headers: Map<String, Value> = headers
        .iter()
        .map(|(name, value)| (name.to_string(), json!(value.to_str().unwrap())))
        .collect();
    let smtp_port = state
        .settings
        .snapshot()
        .entry("smtp.port")
        .map(|entry| entry.value().to_json());
    Json(
        json!({ "uri": uri.to_string(), "headers": headers, "body": body, "smtp_port": smtp_port }),
    )
}

/// Writes through the real settings service, outputs what it wrote.
async fn smtp_port(state: &AppState) -> anyhow::Result<SeedOutputs> {
    let change = Change {
        key: "smtp.port".to_owned(),
        value: json!(DEMO_PORT),
        version: 0,
    };
    state
        .settings
        .update(UpdateRequest {
            changes: vec![change],
            actor: None,
            reason: None,
        })
        .await?;
    Ok(SeedOutputs::from([(
        "smtp_port".to_owned(),
        DEMO_PORT.to_string(),
    )]))
}

async fn token(_: &AppState) -> anyhow::Result<SeedOutputs> {
    Ok(SeedOutputs::from([("token".to_owned(), "t".to_owned())]))
}

async fn failing(_: &AppState) -> anyhow::Result<SeedOutputs> {
    anyhow::bail!("demo seed failure")
}

fn setup() -> Setup {
    Setup {
        seeds: Seeds::builtin()
            .with("smtp-port", |state| Box::pin(smtp_port(state)))
            .with("token", |state| Box::pin(token(state)))
            .with("failing", |state| Box::pin(failing(state))),
        routes: Router::new().route("/test/echo", any(echo)),
    }
}

async fn run_filtered(fixtures: &[(&str, Value)], filter: Option<&str>) -> Outcome {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("UPSTREAM_COMMIT"), format!("{COMMIT}\n")).unwrap();
    for (name, fixture) in fixtures {
        write_fixture(dir.path(), name, fixture);
    }
    run_dir_with(dir.path(), filter, &setup()).await.unwrap()
}

async fn run(fixtures: &[(&str, Value)]) -> Outcome {
    run_filtered(fixtures, None).await
}

#[tokio::test]
async fn unknown_route_matches_the_stack_error_contract() {
    let ok = fixture(
        json!({ "method": "GET", "path": "/api/no-such-endpoint" }),
        json!({ "status": 404, "body_kind": "json", "shape": "ErrorBody",
                "json_subset": { "error": "not found" } }),
    );

    let outcome = run(&[("stack/not-found.json", ok)]).await;

    assert!(outcome.failures.is_empty(), "{outcome}");
    assert_eq!(outcome.passed, ["stack/not-found.json"]);
}

#[tokio::test]
async fn wrong_expectations_fail_with_file_source_and_body() {
    let wrong = fixture(
        json!({ "method": "GET", "path": "/api/no-such-endpoint" }),
        json!({ "status": 200, "body_kind": "json", "shape": "LoginOptions" }),
    );

    let outcome = run(&[("stack/wrong.json", wrong)]).await;
    let report = outcome.to_string();

    assert!(outcome.passed.is_empty());
    assert!(
        report.contains("tests/contract/fixtures/stack/wrong.json"),
        "{report}"
    );
    assert!(report.contains("src/hbbs_http.rs:24-39"), "{report}");
    assert!(report.contains("expected 200, got 404"), "{report}");
    assert!(report.contains("LoginOptions does not parse"), "{report}");
    assert!(
        report.contains(r#"body: {"error":"not found"}"#),
        "{report}"
    );
}

#[tokio::test]
async fn invalid_fixtures_fail_without_hiding_the_others() {
    let mut stale = fixture(
        json!({ "method": "GET", "path": "/api/x" }),
        json!({ "status": 404, "body_kind": "json" }),
    );
    stale["upstream_commit"] = json!("23e5bb4");

    let outcome = run(&[
        ("a/broken.json", json!({ "not": "a fixture" })),
        ("b/stale.json", stale),
        (
            "c/ok.json",
            fixture(
                json!({ "method": "GET", "path": "/api/x" }),
                json!({ "status": 404, "body_kind": "json" }),
            ),
        ),
    ])
    .await;
    let report = outcome.to_string();

    assert_eq!(outcome.passed, ["c/ok.json"]);
    assert_eq!(outcome.failures.len(), 2, "{report}");
    assert!(report.contains("invalid fixture"), "{report}");
    assert!(report.contains("UPSTREAM_COMMIT"), "{report}");
}

#[tokio::test]
async fn request_headers_and_body_are_sent_as_written() {
    let echoed = fixture(
        json!({ "method": "POST", "path": "/test/echo?a=1",
                "headers": { "content-type": "application/json", "X-Client": "rustdesk" },
                "body": { "json": { "id": "1" } } }),
        json!({ "status": 200, "body_kind": "json", "json_subset": {
            "uri": "/test/echo?a=1",
            "headers": { "content-type": "application/json", "x-client": "rustdesk",
                         "content-length": "10" },
            "body": r#"{"id":"1"}"#
        } }),
    );

    let outcome = run(&[("echo/post.json", echoed)]).await;

    assert!(outcome.failures.is_empty(), "{outcome}");
}

#[tokio::test]
async fn seed_outputs_are_substituted_into_path_headers_and_body() {
    let echoed = seeded(
        &["smtp-port"],
        json!({ "method": "POST", "path": "/test/echo?port=${smtp_port}",
                "headers": { "authorization": "Bearer ${smtp_port}" },
                "body": { "json": { "port": ["${smtp_port}", "x${smtp_port}", 1] } } }),
        // smtp_port: the handler sees what the seed wrote (one shared state).
        json!({ "status": 200, "body_kind": "json", "json_subset": {
            "uri": "/test/echo?port=2525",
            "headers": { "authorization": "Bearer 2525" },
            "body": r#"{"port":["2525","x2525",1]}"#,
            "smtp_port": 2525
        } }),
    );

    let outcome = run(&[("echo/seeded.json", echoed)]).await;

    assert!(outcome.failures.is_empty(), "{outcome}");
}

#[tokio::test]
async fn text_bodies_are_substituted_too() {
    let echoed = seeded(
        &["token"],
        json!({ "method": "POST", "path": "/test/echo", "body": { "text": "token=${token}" } }),
        json!({ "status": 200, "body_kind": "json", "json_subset": { "body": "token=t" } }),
    );

    let outcome = run(&[("echo/text.json", echoed)]).await;

    assert!(outcome.failures.is_empty(), "{outcome}");
}

#[tokio::test]
async fn seed_problems_fail_the_fixture_with_the_cause() {
    let any_reply = json!({ "status": 200, "body_kind": "json" });
    let echo = json!({ "method": "GET", "path": "/test/echo" });
    let unresolved = json!({ "method": "GET", "path": "/test/echo?t=${nope}" });

    let outcome = run(&[
        (
            "a/unknown.json",
            seeded(&["no-such-seed"], echo.clone(), any_reply.clone()),
        ),
        (
            "b/failing.json",
            seeded(&["failing"], echo.clone(), any_reply.clone()),
        ),
        (
            "c/twice.json",
            seeded(&["token", "token"], echo, any_reply.clone()),
        ),
        (
            "d/unresolved.json",
            seeded(&["token"], unresolved, any_reply),
        ),
    ])
    .await;
    let report = outcome.to_string();

    assert!(outcome.passed.is_empty(), "{report}");
    assert_eq!(outcome.failures.len(), 4, "{report}");
    assert!(
        report.contains(r#"unknown seed "no-such-seed""#),
        "{report}"
    );
    assert!(report.contains(r#"seed "failing" failed"#), "{report}");
    assert!(report.contains("demo seed failure"), "{report}");
    assert!(
        report.contains(r#"seed output "token" is defined twice"#),
        "{report}"
    );
    assert!(report.contains("${nope}"), "{report}");
}

#[tokio::test]
async fn non_json_files_in_fixture_dirs_fail_discovery() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("UPSTREAM_COMMIT"), COMMIT).unwrap();
    let ok = fixture(
        json!({ "method": "GET", "path": "/api/x" }),
        json!({ "status": 404, "body_kind": "json" }),
    );
    write_fixture(dir.path(), "login/ok.json", &ok);
    std::fs::write(dir.path().join("fixtures/login/case.jsn"), "{}").unwrap();

    let err = run_dir_with(dir.path(), None, &setup()).await.unwrap_err();

    let message = format!("{err:#}");
    assert!(message.contains("login/case.jsn"), "{message}");
    assert!(message.contains(".json"), "{message}");
}

#[tokio::test]
async fn filter_selects_by_relative_path() {
    let ok = fixture(
        json!({ "method": "GET", "path": "/api/x" }),
        json!({ "status": 404, "body_kind": "json" }),
    );
    let broken = json!({ "not": "a fixture" });

    let outcome = run_filtered(
        &[("login/ok.json", ok), ("other/broken.json", broken)],
        Some("login/"),
    )
    .await;

    assert_eq!(outcome.passed, ["login/ok.json"]);
    assert!(outcome.failures.is_empty(), "{outcome}");
}
