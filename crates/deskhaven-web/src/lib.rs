//! Compose into the admin side with `admin_api.merge(deskhaven_web::router())` and set
//! `HttpConfig::content_security_policy` to [`CSP`].
//!
//! API paths (judged on the decoded path with empty segments collapsed) are a bare 404 here,
//! so the `deskhaven-http` API renderers turn them into the API's JSON 404. Release builds
//! embed `web/dist`; without it the SPA routes answer 503.

mod accept;
mod csp;
mod file;
mod not_built;
mod request_path;
mod serve;

use axum::Router;
use rust_embed::RustEmbed;

pub use csp::{CSP, csp_connecting_to};

#[derive(RustEmbed)]
#[folder = "../../web/dist"]
#[allow_missing = true]
#[exclude = "*.map"]
#[exclude = ".*"]
#[exclude = "*/.*"]
struct Dist;

/// All paths are served by its fallback, so merge it into a router that has none.
pub fn router() -> Router {
    serve::router::<Dist>()
}

pub fn is_built() -> bool {
    Dist::get(serve::INDEX).is_some()
}

pub fn router_for<E: RustEmbed + 'static>() -> Router {
    serve::router::<E>()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vacuous when `web/dist` is not built.
    #[test]
    fn the_embed_has_no_dotfiles_or_source_maps() {
        for name in Dist::iter() {
            let hidden = name.split('/').any(|s| s.starts_with('.'));
            assert!(!hidden && !name.ends_with(".map"), "{name}");
        }
    }
}
