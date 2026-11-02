//! `ClientApiError` -> status + `{"error": "<msg>"}` (03-client-api-contract §1.3).
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::io;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use deskhaven_client_api::ClientApiError;
use deskhaven_http::{ErrorBodies, HttpConfig, X_REQUEST_ID, apply_middleware};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn json_of(res: Response) -> (StatusCode, Value) {
    let status = res.status();
    assert_eq!(res.headers()[header::CONTENT_TYPE], "application/json");
    let body = to_bytes(res.into_body(), 4096).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn user_facing_errors_map_to_status_and_message() {
    let cases = [
        (
            ClientApiError::BadRequest("bad input".into()),
            StatusCode::BAD_REQUEST,
        ),
        (
            ClientApiError::Unauthorized("Invalid token".into()),
            StatusCode::UNAUTHORIZED,
        ),
        (
            ClientApiError::Forbidden("Not allowed".into()),
            StatusCode::FORBIDDEN,
        ),
        (
            ClientApiError::NotFound("No such peer".into()),
            StatusCode::NOT_FOUND,
        ),
    ];
    for (err, status) in cases {
        let message = err.to_string();
        assert_eq!(err.status(), status);
        let got = json_of(err.into_response()).await;
        assert_eq!(got, (status, json!({ "error": message })));
    }
}

#[tokio::test]
async fn internal_error_never_leaks_detail() {
    let err = ClientApiError::internal(io::Error::other("db password=hunter2"));
    let got = json_of(err.into_response()).await;
    assert_eq!(
        got,
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": "internal server error" })
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
        get(|| async { Err::<(), _>(ClientApiError::internal(io::Error::other("disk on fire"))) }),
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
