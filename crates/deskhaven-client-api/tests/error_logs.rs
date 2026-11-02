//! Internal error details reach the log with the request id. Its own test binary:
//! `tracing` caches callsite interest process-wide, so a scoped subscriber races with
//! tests running in parallel in the same binary.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::io;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use deskhaven_client_api::ClientApiError;
use deskhaven_http::{ErrorBodies, HttpConfig, X_REQUEST_ID, apply_middleware};
use tower::ServiceExt;

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
