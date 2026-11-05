//! Health probes run every few seconds; their per-request log line is debug,
//! everything else stays info. Its own test binary: `tracing` caches callsite
//! interest process-wide, so a scoped subscriber races with parallel tests.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::io;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::routing::get;
use deskhaven_http::{ErrorBodies, HttpConfig, apply_middleware, health_routes};
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
async fn probe_requests_are_not_logged_at_info() {
    let logs = Capture::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let routes = health_routes(|| async { Ok(()) })
        .merge(Router::new().route("/api/thing", get(|| async { "ok" })));
    let app = apply_middleware(routes, &HttpConfig::default(), ErrorBodies::default());

    for path in ["/healthz", "/readyz", "/api/thing"] {
        let req = Request::get(path).body(Body::empty()).unwrap();
        app.clone().oneshot(req).await.unwrap();
    }

    let text = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    let finished: Vec<&str> = text
        .lines()
        .filter(|l| l.contains("request finished"))
        .collect();
    assert_eq!(finished.len(), 1, "{text}");
    assert!(finished[0].contains("path=/api/thing"), "{text}");
}
