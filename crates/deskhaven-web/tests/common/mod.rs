//! Asset sets and an in-process request helper shared by the integration tests.
// Each test binary uses a different subset; unwrap is fine in test helpers.
#![allow(dead_code, clippy::unwrap_used, clippy::panic)]

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use rust_embed::RustEmbed;
use tower::ServiceExt;

/// A small stand-in for `web/dist` (the real one may not be built).
#[derive(RustEmbed)]
#[folder = "tests/fixtures/dist"]
pub struct Fixture;

/// A dist that does not exist: the "built without the frontend" case.
#[derive(RustEmbed)]
#[folder = "tests/fixtures/missing"]
#[allow_missing = true]
pub struct Missing;

pub const INDEX_HTML: &str = include_str!("../fixtures/dist/index.html");

pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: String,
}

impl Reply {
    pub fn header(&self, name: &str) -> &str {
        self.headers
            .get(name)
            .unwrap_or_else(|| panic!("missing header {name}: {:?}", self.headers))
            .to_str()
            .unwrap()
    }
}

pub struct Query<'a> {
    pub method: Method,
    pub path: &'a str,
    pub headers: &'a [(&'a str, &'a str)],
}

pub fn get<'a>(path: &'a str) -> Query<'a> {
    Query {
        method: Method::GET,
        path,
        headers: &[],
    }
}

impl<'a> Query<'a> {
    pub fn method(self, method: Method) -> Self {
        Self { method, ..self }
    }

    pub fn headers(self, headers: &'a [(&'a str, &'a str)]) -> Self {
        Self { headers, ..self }
    }

    pub async fn send(self, router: &Router) -> Reply {
        let mut builder = Request::builder().method(self.method).uri(self.path);
        for (name, value) in self.headers {
            builder = builder.header(*name, *value);
        }
        let res = router
            .clone()
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let (parts, body) = res.into_parts();
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        Reply {
            status: parts.status,
            headers: parts.headers,
            body: String::from_utf8_lossy(&bytes).into_owned(),
        }
    }
}
