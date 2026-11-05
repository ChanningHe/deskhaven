//! `GET /healthz` (liveness, always 200) and `GET /readyz` (readiness, pluggable).

use std::sync::Arc;

use axum::Router;
use axum::http::StatusCode;
use axum::routing::get;

use crate::paths;

/// Why the service is not ready. Logged, never sent to the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotReady(String);

impl NotReady {
    pub fn new(reason: impl Into<String>) -> Self {
        Self(reason.into())
    }

    pub fn reason(&self) -> &str {
        &self.0
    }
}

/// Readiness probe (the bin plugs in DB reachability + migration status).
/// Implemented for any `Fn() -> impl Future<Output = Result<(), NotReady>>`.
pub trait ReadinessCheck: Send + Sync + 'static {
    fn check(&self) -> impl Future<Output = Result<(), NotReady>> + Send;
}

impl<F, Fut> ReadinessCheck for F
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), NotReady>> + Send,
{
    fn check(&self) -> impl Future<Output = Result<(), NotReady>> + Send {
        self()
    }
}

pub fn health_routes<S, R>(readiness: R) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    R: ReadinessCheck,
{
    let readiness = Arc::new(readiness);
    Router::new()
        .route(paths::HEALTHZ, get(|| async { "ok" }))
        .route(paths::READYZ, get(move || readyz(Arc::clone(&readiness))))
}

async fn readyz<R: ReadinessCheck>(readiness: Arc<R>) -> (StatusCode, &'static str) {
    match readiness.check().await {
        Ok(()) => (StatusCode::OK, "ok"),
        Err(not_ready) => {
            tracing::warn!(reason = not_ready.reason(), "readiness check failed");
            (StatusCode::SERVICE_UNAVAILABLE, "not ready")
        }
    }
}
