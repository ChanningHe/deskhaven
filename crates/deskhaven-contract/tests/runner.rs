//! The runner must fail loudly when a fixture is wrong: these fixtures live
//! in a temporary contract directory and are expected to fail.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::Path;

use deskhaven_contract::runner::{Outcome, run_dir};
use serde_json::{Value, json};

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

fn write_fixture(dir: &Path, name: &str, fixture: &Value) {
    let path = dir.join("fixtures").join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, fixture.to_string()).unwrap();
}

fn fixture(request: Value, expect: Value) -> Value {
    json!({
        "upstream_commit": COMMIT,
        "source": ["src/hbbs_http.rs:25-32"],
        "description": "runner self-test",
        "request": request,
        "expect": expect
    })
}

async fn run(fixtures: &[(&str, Value)]) -> Outcome {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("UPSTREAM_COMMIT"), format!("{COMMIT}\n")).unwrap();
    for (name, fixture) in fixtures {
        write_fixture(dir.path(), name, fixture);
    }
    run_dir(dir.path(), None).await.unwrap()
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
    assert!(report.contains("src/hbbs_http.rs:25-32"), "{report}");
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
    // A JSON body on a GET route: the route ignores it, but it must not
    // break request building, and content-length must be accepted.
    let ok = fixture(
        json!({ "method": "POST", "path": "/api/no-such-endpoint",
                "headers": { "content-type": "application/json" },
                "body": { "json": { "id": "1" } } }),
        json!({ "status": 404, "body_kind": "json", "json_absent": ["/data"] }),
    );

    let outcome = run(&[("stack/post.json", ok)]).await;

    assert!(outcome.failures.is_empty(), "{outcome}");
}
