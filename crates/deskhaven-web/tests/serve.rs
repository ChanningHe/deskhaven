//! Asset serving, caching and the SPA fallback, against the fixture dist.

mod common;

use axum::http::{Method, StatusCode};
use common::{Fixture, INDEX_HTML, Missing, get};
use deskhaven_web::router_for;

const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const ACCEPT_NAV: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";

fn router() -> axum::Router {
    router_for::<Fixture>()
}

#[tokio::test]
async fn serves_a_hashed_asset_with_type_etag_and_immutable_caching() {
    let res = get("/assets/index-AbC123.js").send(&router()).await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.header("content-type"), "text/javascript; charset=utf-8");
    assert_eq!(res.header("cache-control"), IMMUTABLE);
    assert!(res.header("etag").starts_with('"'));
    assert_eq!(res.body, "console.log(\"fixture app\")\n");
}

#[tokio::test]
async fn asset_content_types_follow_the_extension() {
    let r = router();
    let css = get("/assets/index-XyZ789.css").send(&r).await;
    let font = get("/assets/font-Q1w2E3.woff2").send(&r).await;
    let svg = get("/favicon.svg").send(&r).await;

    assert_eq!(css.header("content-type"), "text/css; charset=utf-8");
    assert_eq!(font.header("content-type"), "font/woff2");
    assert_eq!(svg.header("content-type"), "image/svg+xml");
}

#[tokio::test]
async fn non_hashed_files_and_index_must_revalidate() {
    let r = router();
    for path in ["/theme-init.js", "/favicon.svg", "/index.html", "/"] {
        let res = get(path).send(&r).await;
        assert_eq!(res.status, StatusCode::OK, "{path}");
        assert_eq!(res.header("cache-control"), "no-cache", "{path}");
    }
}

#[tokio::test]
async fn if_none_match_with_the_current_etag_is_304_without_a_body() {
    let r = router();
    let first = get("/assets/index-AbC123.js").send(&r).await;
    let etag = first.header("etag").to_owned();

    let res = get("/assets/index-AbC123.js")
        .headers(&[("if-none-match", &etag)])
        .send(&r)
        .await;

    assert_eq!(res.status, StatusCode::NOT_MODIFIED);
    assert_eq!(res.header("etag"), etag);
    assert_eq!(res.header("cache-control"), IMMUTABLE);
    assert!(res.body.is_empty());
}

#[tokio::test]
async fn a_stale_etag_gets_the_full_body() {
    let res = get("/assets/index-AbC123.js")
        .headers(&[("if-none-match", "\"stale\"")])
        .send(&router())
        .await;

    assert_eq!(res.status, StatusCode::OK);
    assert!(!res.body.is_empty());
}

#[tokio::test]
async fn the_spa_fallback_serves_index_for_client_routes() {
    let r = router();
    for path in ["/devices", "/settings/x", "/settings/x/", "/login?next=%2F"] {
        let res = get(path).headers(&[("accept", ACCEPT_NAV)]).send(&r).await;
        assert_eq!(res.status, StatusCode::OK, "{path}");
        assert_eq!(res.header("content-type"), "text/html; charset=utf-8");
        assert_eq!(res.header("cache-control"), "no-cache");
        assert_eq!(res.body, INDEX_HTML, "{path}");
    }
}

#[tokio::test]
async fn the_spa_fallback_works_without_an_accept_header() {
    let res = get("/devices").send(&router()).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body, INDEX_HTML);
}

#[tokio::test]
async fn the_fallback_shares_the_index_etag() {
    let r = router();
    let index = get("/index.html").send(&r).await;
    let fallback = get("/devices").send(&r).await;
    assert_eq!(index.header("etag"), fallback.header("etag"));
}

#[tokio::test]
async fn non_html_clients_do_not_get_the_spa() {
    let res = get("/devices")
        .headers(&[("accept", "application/json")])
        .send(&router())
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_missing_file_with_an_extension_is_404_not_html() {
    let r = router();
    for path in ["/assets/x.js", "/nope.css", "/assets/index-AbC123.js.map"] {
        let res = get(path).headers(&[("accept", ACCEPT_NAV)]).send(&r).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{path}");
        assert!(!res.body.contains("<html"), "{path}");
    }
}

#[tokio::test]
async fn api_prefixes_are_never_served_the_spa() {
    let r = router();
    for path in [
        "/api",
        "/api/unknown",
        "/admin/api",
        "/admin/api/x",
        "/%61pi/x",
        "//api/x",
        "/%2fapi/x",
        "/admin//api/x",
        "/admin%2Fapi/x",
    ] {
        let res = get(path).headers(&[("accept", ACCEPT_NAV)]).send(&r).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{path}");
        assert!(res.body.is_empty(), "{path}");
    }
}

#[tokio::test]
async fn paths_that_only_look_like_api_prefixes_get_the_spa() {
    let r = router();
    for path in ["/apix", "/admin", "/admin/settings", "/admin/apix"] {
        assert_eq!(get(path).send(&r).await.status, StatusCode::OK, "{path}");
    }
}

#[tokio::test]
async fn directories_are_not_listed() {
    let r = router();
    for path in ["/assets", "/assets/"] {
        let res = get(path).send(&r).await;
        // No extension: the SPA's client router decides. Never a listing.
        assert_eq!(res.body, INDEX_HTML, "{path}");
    }
}

#[tokio::test]
async fn traversal_is_404_in_every_encoding() {
    let r = router();
    for path in [
        "/..",
        "/../Cargo.toml",
        "/assets/../index.html",
        "/assets/%2e%2e/index.html",
        "/%2e%2e/%2e%2e/etc/passwd",
        "/assets/..%2findex.html",
        "/assets/%2E%2E%5Cindex.html",
        "/.env",
        "/%00",
        "/%zz",
    ] {
        let res = get(path).headers(&[("accept", ACCEPT_NAV)]).send(&r).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn head_matches_get_headers_with_an_empty_body() {
    let r = router();
    let res = get("/assets/index-AbC123.js")
        .method(Method::HEAD)
        .send(&r)
        .await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.header("cache-control"), IMMUTABLE);
    assert!(res.body.is_empty());
}

#[tokio::test]
async fn other_methods_are_405_with_allow() {
    let r = router();
    for method in [Method::POST, Method::PUT, Method::DELETE, Method::PATCH] {
        let res = get("/devices").method(method.clone()).send(&r).await;
        assert_eq!(res.status, StatusCode::METHOD_NOT_ALLOWED, "{method}");
        assert_eq!(res.header("allow"), "GET, HEAD");
    }
}

#[tokio::test]
async fn posting_under_an_api_prefix_is_404_not_405() {
    let res = get("/admin/api/x")
        .method(Method::POST)
        .send(&router())
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn without_a_dist_spa_routes_say_the_console_is_not_built() {
    let r = router_for::<Missing>();

    let res = get("/devices").send(&r).await;
    assert_eq!(res.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(res.header("content-type"), "text/html; charset=utf-8");
    assert!(res.body.contains("web console is not built"));
    assert!(res.body.contains("xtask -- dist"));
    assert!(!res.body.contains("<script"));

    let res = get("/").send(&r).await;
    assert_eq!(res.status, StatusCode::SERVICE_UNAVAILABLE);
    let res = get("/assets/x.js").send(&r).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

/// `router()` is wired to the real `web/dist`, which may or may not be built here.
#[tokio::test]
async fn the_embedded_router_serves_the_spa_or_the_not_built_page() {
    let r = deskhaven_web::router();

    let spa = get("/devices").send(&r).await;
    let status = spa.status;
    assert!(
        status == StatusCode::OK || status == StatusCode::SERVICE_UNAVAILABLE,
        "{status}"
    );
    assert!(spa.body.contains("<html"));
    let api = get("/api/unknown").send(&r).await;
    assert_eq!(api.status, StatusCode::NOT_FOUND);
}
