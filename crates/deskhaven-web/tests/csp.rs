//! The CSP and the markup it has to admit: the SPA entry point must work under
//! it without inline script or style.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};

use deskhaven_web::CSP;

fn repo_file(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// What in `html` a strict CSP (`script-src 'self'; style-src 'self'`) blocks.
fn csp_violations(html: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let mut found = Vec::new();
    for tag in tags(&lower) {
        let name = tag.split_whitespace().next().unwrap_or_default();
        match name {
            "script" if !has_attr(tag, "src") => found.push(format!("inline script: <{tag}>")),
            "style" => found.push("<style> element".to_owned()),
            _ => {}
        }
        if has_attr(tag, "style") {
            found.push(format!("style attribute: <{tag}>"));
        }
        if tag
            .split_whitespace()
            .any(|w| w.starts_with("on") && w.contains('='))
        {
            found.push(format!("event handler: <{tag}>"));
        }
        if tag.contains("javascript:") {
            found.push(format!("javascript: URL: <{tag}>"));
        }
    }
    found
}

/// Opening-tag contents (between `<` and `>`), skipping comments.
fn tags(html: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        rest = &rest[start..];
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = after.split_once("-->").map_or("", |(_, tail)| tail);
            continue;
        }
        let Some(end) = rest.find('>') else { break };
        let tag = &rest[1..end];
        if !tag.starts_with('/') && !tag.starts_with('!') {
            out.push(tag);
        }
        rest = &rest[end + 1..];
    }
    out
}

fn has_attr(tag: &str, name: &str) -> bool {
    // The first word is the tag name, not an attribute.
    tag.split_whitespace()
        .skip(1)
        .any(|w| w == name || w.starts_with(&format!("{name}=")))
}

#[test]
fn the_scanner_flags_what_the_csp_blocks() {
    let bad = r#"<script>alert(1)</script><p style="x:y"><style>a{}</style><a onclick="f()">"#;
    let found = csp_violations(bad);
    assert_eq!(found.len(), 4, "{found:?}");
}

#[test]
fn the_scanner_accepts_external_scripts_and_ignores_comments() {
    let ok = r#"<!-- <script>alert(1)</script> --><script src="/a.js"></script>
        <script type="module" crossorigin src="/b.js"></script><link rel="stylesheet" href="/c.css">"#;
    assert_eq!(csp_violations(ok), Vec::<String>::new());
}

#[test]
fn the_spa_source_entry_point_is_csp_clean() {
    let html = std::fs::read_to_string(repo_file("web/index.html")).unwrap();
    assert_eq!(csp_violations(&html), Vec::<String>::new());
}

/// Skipped (with a note) when the frontend has not been built; `xtask dist`
/// and CI build it first.
#[test]
fn the_built_index_is_csp_clean() {
    let path = repo_file("web/dist/index.html");
    let Ok(html) = std::fs::read_to_string(&path) else {
        eprintln!("skipping: {} not built", path.display());
        return;
    };
    assert_eq!(csp_violations(&html), Vec::<String>::new());
}

#[test]
fn the_built_css_needs_no_data_urls_or_imports() {
    let assets = repo_file("web/dist/assets");
    let Ok(entries) = std::fs::read_dir(&assets) else {
        eprintln!("skipping: {} not built", assets.display());
        return;
    };
    for entry in entries.map(Result::unwrap) {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "css") {
            let css = std::fs::read_to_string(&path).unwrap();
            assert!(!css.contains("data:"), "{}", path.display());
            assert!(!css.contains("@import"), "{}", path.display());
        }
    }
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
