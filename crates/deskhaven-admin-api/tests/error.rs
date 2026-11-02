//! `AdminApiError` -> status + `{error:{code,message,details?}}` (08-web-console §6).
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::io;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use deskhaven_admin_api::{AdminApiError, ErrorCode};
use deskhaven_http::{ErrorBodies, HttpConfig, X_REQUEST_ID, apply_middleware};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn json_of(res: Response) -> (StatusCode, Value) {
    let status = res.status();
    assert_eq!(res.headers()[header::CONTENT_TYPE], "application/json");
    let body = to_bytes(res.into_body(), 4096).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[test]
fn codes_are_stable_snake_case_with_fixed_statuses() {
    let cases = [
        (
            ErrorCode::ValidationFailed,
            "validation_failed",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            ErrorCode::Unauthorized,
            "unauthorized",
            StatusCode::UNAUTHORIZED,
        ),
        (ErrorCode::Forbidden, "forbidden", StatusCode::FORBIDDEN),
        (ErrorCode::NotFound, "not_found", StatusCode::NOT_FOUND),
        (ErrorCode::Conflict, "conflict", StatusCode::CONFLICT),
        (
            ErrorCode::Internal,
            "internal",
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
    ];
    for (code, wire, status) in cases {
        assert_eq!(serde_json::to_value(code).unwrap(), json!(wire));
        assert_eq!(code.as_str(), wire);
        assert_eq!(code.status(), status);
    }
}

#[tokio::test]
async fn error_without_details_omits_the_field() {
    let got = json_of(AdminApiError::not_found("user not found").into_response()).await;
    assert_eq!(
        got,
        (
            StatusCode::NOT_FOUND,
            json!({ "error": { "code": "not_found", "message": "user not found" } })
        )
    );
}

#[tokio::test]
async fn validation_error_carries_details() {
    let details = json!({ "fields": { "email": ["invalid"] } });
    let err = AdminApiError::validation_failed("invalid input").with_details(details.clone());
    let got = json_of(err.into_response()).await;
    let expected = json!({
        "error": { "code": "validation_failed", "message": "invalid input", "details": details }
    });
    assert_eq!(got, (StatusCode::UNPROCESSABLE_ENTITY, expected));
}

#[tokio::test]
async fn convenience_constructors_use_matching_codes() {
    let cases = [
        (AdminApiError::unauthorized(), ErrorCode::Unauthorized),
        (AdminApiError::forbidden(), ErrorCode::Forbidden),
        (
            AdminApiError::conflict("version mismatch"),
            ErrorCode::Conflict,
        ),
        (
            AdminApiError::new(ErrorCode::NotFound, "gone"),
            ErrorCode::NotFound,
        ),
    ];
    for (err, code) in cases {
        assert_eq!(err.code(), code);
        let (status, body) = json_of(err.into_response()).await;
        assert_eq!(status, code.status());
        assert_eq!(body["error"]["code"], json!(code.as_str()));
        assert!(
            body["error"]["message"]
                .as_str()
                .is_some_and(|m| !m.is_empty())
        );
    }
}

#[tokio::test]
async fn internal_error_never_leaks_detail() {
    let err = AdminApiError::internal(io::Error::other("db password=hunter2"));
    let got = json_of(err.into_response()).await;
    assert_eq!(
        got,
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": { "code": "internal", "message": "internal server error" } })
        )
    );
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn internal_error_detail_is_logged_with_request_id() {
    let logs = Capture::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let routes = Router::new().route(
        "/fail",
        get(|| async { Err::<(), _>(AdminApiError::internal(io::Error::other("disk on fire"))) }),
    );
    let req = Request::get("/fail").body(Body::empty()).unwrap();
    let res = apply_middleware(routes, &HttpConfig::default(), ErrorBodies::default())
        .oneshot(req)
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let id = res.headers()[X_REQUEST_ID].to_str().unwrap().to_owned();

    let text = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    let line = text
        .lines()
        .find(|l| l.contains("disk on fire"))
        .unwrap_or_else(|| panic!("detail not logged: {text}"));
    assert!(line.contains(&format!("request_id={id}")), "{line}");
}
