//! Liveness and readiness routes.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use deskhaven_http::{NotReady, health_routes};
use tower::ServiceExt;

async fn get(app: Router, path: &str) -> (StatusCode, String) {
    let res = app
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let body = to_bytes(res.into_body(), 1024).await.unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

fn app_with_flag(flag: Arc<AtomicBool>) -> Router {
    health_routes(move || {
        let ready = flag.load(Ordering::SeqCst);
        async move {
            if ready {
                Ok(())
            } else {
                Err(NotReady::new("migrations pending: secret dsn"))
            }
        }
    })
}

#[tokio::test]
async fn healthz_is_always_ok() {
    let app = app_with_flag(Arc::new(AtomicBool::new(false)));
    assert_eq!(
        get(app, "/healthz").await,
        (StatusCode::OK, "ok".to_owned())
    );
}

#[tokio::test]
async fn readyz_is_200_when_check_passes() {
    let app = app_with_flag(Arc::new(AtomicBool::new(true)));
    assert_eq!(get(app, "/readyz").await, (StatusCode::OK, "ok".to_owned()));
}

#[tokio::test]
async fn readyz_is_503_without_leaking_reason() {
    let app = app_with_flag(Arc::new(AtomicBool::new(false)));
    let (status, body) = get(app, "/readyz").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body, "not ready");
}

#[tokio::test]
async fn readyz_reflects_current_state() {
    let flag = Arc::new(AtomicBool::new(false));
    let app = app_with_flag(flag.clone());
    assert_eq!(
        get(app.clone(), "/readyz").await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    flag.store(true, Ordering::SeqCst);
    assert_eq!(get(app, "/readyz").await.0, StatusCode::OK);
}
