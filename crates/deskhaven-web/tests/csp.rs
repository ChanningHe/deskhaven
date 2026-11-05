//! The CSP and the markup it has to admit: the SPA entry point must work under
//! it without inline script or style.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]

// Not `tests/scan.rs`: cargo would build that as a test target of its own.
#[path = "csp/scan.rs"]
mod scan;

use std::path::{Path, PathBuf};

use deskhaven_web::CSP;
use scan::{csp_violations, start_tags};

fn repo_file(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn none() -> Vec<String> {
    Vec::new()
}

/// Set to `1` by the release pipeline: a missing `web/dist` then fails the built-dist
/// checks instead of skipping them (normal CI does not build the console for the Rust
/// jobs). Same convention as `DESKHAVEN_TEST_REQUIRE_PG`.
const REQUIRE_DIST_ENV: &str = "DESKHAVEN_TEST_REQUIRE_WEB_DIST";

fn dist_required() -> bool {
    std::env::var(REQUIRE_DIST_ENV).is_ok_and(|v| v == "1")
}

/// The frontend is not built: skip with a note, or panic when a build is required.
fn dist_missing(required: bool, path: &Path) {
    assert!(
        !required,
        "{REQUIRE_DIST_ENV}=1 but {} is not built",
        path.display()
    );
    eprintln!("skipping: {} not built", path.display());
}

#[test]
fn a_missing_dist_is_skipped_unless_required() {
    dist_missing(false, Path::new("web/dist"));
}

#[test]
#[should_panic(expected = "is not built")]
fn a_missing_dist_fails_when_required() {
    dist_missing(true, Path::new("web/dist"));
}

#[test]
fn the_scanner_flags_what_the_csp_blocks() {
    let bad = r#"<script>alert(1)</script><p style="x:y"><style>a{}</style><a onclick="f()">"#;
    assert_eq!(csp_violations(bad).len(), 4, "{:?}", csp_violations(bad));
}

#[test]
fn the_scanner_sees_spaced_unquoted_and_upper_case_attributes() {
    for bad in [
        r#"<p style = "x:y">"#,
        "<p style=x:y>",
        "<P STYLE='x:y'>",
        r#"<a ONCLICK = "f()">"#,
        r#"<a href=" JavaScript:alert(1)">"#,
        "<SCRIPT>alert(1)</SCRIPT>",
        "<script\n>alert(1)</script>",
    ] {
        assert_eq!(
            csp_violations(bad).len(),
            1,
            "{bad}: {:?}",
            csp_violations(bad)
        );
    }
}

#[test]
fn a_gt_inside_an_attribute_value_does_not_end_the_tag() {
    // The style attribute comes after a `>` in `alt`.
    let hidden = r#"<img alt="a>b" style="x:y">"#;
    assert_eq!(
        csp_violations(hidden).len(),
        1,
        "{:?}",
        csp_violations(hidden)
    );
    // Markup-looking text inside a value is text, not a tag.
    let text = r#"<div title="x > <script>alert(1)</script> onclick=y">"#;
    assert_eq!(csp_violations(text), none());
    assert_eq!(start_tags(text).len(), 1);
}

#[test]
fn the_scanner_accepts_external_scripts_and_ignores_comments() {
    let ok = r#"<!doctype html><!-- <script>alert(1)</script> --><script src="/a.js"></script>
        <script type="module" crossorigin src="/b.js"></script>
        <link rel="stylesheet" href="/c.css"><meta name="x" content="one">"#;
    assert_eq!(csp_violations(ok), none());
}

#[test]
fn the_spa_source_entry_point_is_csp_clean() {
    let html = std::fs::read_to_string(repo_file("web/index.html")).unwrap();
    assert_eq!(csp_violations(&html), none());
    assert!(!start_tags(&html).is_empty());
}

/// Skipped (with a note) when the frontend has not been built, unless
/// `DESKHAVEN_TEST_REQUIRE_WEB_DIST=1` (the release pipeline) turns that into a failure.
#[test]
fn the_built_index_is_csp_clean() {
    let path = repo_file("web/dist/index.html");
    let Ok(html) = std::fs::read_to_string(&path) else {
        dist_missing(dist_required(), &path);
        return;
    };
    assert_eq!(csp_violations(&html), none());
    // An empty or tag-less file would pass the scan vacuously.
    assert!(
        !start_tags(&html).is_empty(),
        "{} has no tags",
        path.display()
    );
}

#[test]
fn the_built_css_needs_no_data_urls_or_imports() {
    let assets = repo_file("web/dist/assets");
    let Ok(entries) = std::fs::read_dir(&assets) else {
        dist_missing(dist_required(), &assets);
        return;
    };
    let mut stylesheets = 0;
    for entry in entries.map(Result::unwrap) {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "css") {
            stylesheets += 1;
            let css = std::fs::read_to_string(&path).unwrap();
            assert!(!css.contains("data:"), "{}", path.display());
            assert!(!css.contains("@import"), "{}", path.display());
        }
    }
    // A required build without any stylesheet would pass vacuously.
    assert!(
        stylesheets > 0 || !dist_required(),
        "no stylesheet in {}",
        assets.display()
    );
}

#[test]
fn the_policy_is_strict_and_a_valid_header_value() {
    let directives: Vec<&str> = CSP.split("; ").collect();
    for required in [
        "default-src 'self'",
        "script-src 'self'",
        "object-src 'none'",
        "frame-ancestors 'none'",
        "base-uri 'none'",
        "form-action 'self'",
    ] {
        assert!(directives.contains(&required), "missing {required}");
    }
    assert!(!CSP.contains("unsafe-"), "{CSP}");
    assert!(!CSP.contains('*'), "{CSP}");
    assert!(axum::http::HeaderValue::from_str(CSP).is_ok());
}
