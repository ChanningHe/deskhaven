//! Embedded web console: serves `web/dist` (the React SPA) from the binary.
//!
//! Compose it into the admin side with `admin_api.merge(deskhaven_web::router())`
//! and give `deskhaven-http` the policy:
//!
//! ```text
//! let cfg = HttpConfig {
//!     content_security_policy: Some(HeaderValue::from_static(deskhaven_web::CSP)),
//!     ..HttpConfig::default()
//! };
//! ```
//!
//! # Behaviour
//!
//! - An exact file is served with its `Content-Type`, a strong `ETag`
//!   (`If-None-Match` -> 304) and `Cache-Control`: `public, max-age=31536000,
//!   immutable` under `assets/` (Vite fingerprints those names), `no-cache`
//!   for everything else, including `index.html`.
//! - SPA fallback: a GET/HEAD for a path that is not a file, has no file
//!   extension, is not under `/api` or `/admin/api` (whole segments, percent
//!   decoded) and accepts HTML gets `index.html` (200). A missing file with an
//!   extension (`/assets/x.js`) or a non-HTML `Accept` is a 404. Directories
//!   are never listed.
//! - API paths are a bare 404 here, so the stack's API renderers
//!   (`deskhaven-http`) turn them into the API's JSON 404.
//! - Other methods get 405. `..`, hidden segments and malformed escapes are 404.
//! - Security headers other than the cache headers come from the
//!   `deskhaven-http` middleware.
//!
//! # Build
//!
//! Release builds embed `web/dist` (debug builds read it from disk). Without
//! `web/dist` the crate still compiles, and the SPA routes answer 503 with a
//! "web console not built" page. `cargo run -p xtask -- dist` builds the
//! frontend and then the release binary.

mod accept;
mod csp;
mod file;
mod not_built;
mod request_path;
mod serve;

use axum::Router;
use rust_embed::RustEmbed;

pub use csp::CSP;

/// The console assets compiled in from `web/dist`.
#[derive(RustEmbed)]
#[folder = "../../web/dist"]
#[allow_missing = true]
#[exclude = "*.map"]
struct Dist;

/// Router for the embedded console. All paths are served by its fallback, so
/// merge it into a router that has none.
pub fn router() -> Router {
    serve::router::<Dist>()
}

/// Like [`router`] for another asset set (tests, alternative dists).
pub fn router_for<E: RustEmbed + 'static>() -> Router {
    serve::router::<E>()
}
