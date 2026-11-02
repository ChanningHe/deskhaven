//! Request path -> what the web console may serve for it.
//!
//! The path is percent-decoded first so that encoded traversal (`%2e%2e`,
//! `%2f`) and an encoded API prefix (`/%61pi/x`) are judged on what they mean.

/// Whole-segment API prefixes (the same ones `deskhaven-http` re-renders errors for).
const API_PREFIXES: [&str; 2] = ["/api", "/admin/api"];

/// What to do with a request path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    /// Never served: undecodable, traversal, hidden file, or an API path
    /// (the API owns its 404, the SPA must not swallow it).
    Reject,
    /// A candidate file key (`assets/a.js`; empty for `/`).
    Path { key: String, has_extension: bool },
}

pub(crate) fn classify(raw_path: &str) -> Target {
    let Some(path) = percent_decode(raw_path) else {
        return Target::Reject;
    };
    if is_api(&path) {
        return Target::Reject;
    }
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.iter().any(|s| is_forbidden_segment(s)) {
        return Target::Reject;
    }
    let has_extension = segments.last().is_some_and(|s| s.contains('.'));
    Target::Path {
        key: segments.join("/"),
        has_extension,
    }
}

fn is_api(path: &str) -> bool {
    API_PREFIXES.iter().any(|prefix| {
        path.strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    })
}

/// `.`/`..`, hidden files, and characters that are path syntax elsewhere.
fn is_forbidden_segment(segment: &str) -> bool {
    segment.starts_with('.') || segment.contains(['\\', '\0'])
}

/// `None` when a `%` escape is malformed or the result is not UTF-8.
fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let byte = *bytes.get(i)?;
        if byte != b'%' {
            out.push(byte);
            i += 1;
            continue;
        }
        let hi = hex_digit(*bytes.get(i + 1)?)?;
        let lo = hex_digit(*bytes.get(i + 2)?)?;
        out.push((hi << 4) | lo);
        i += 3;
    }
    String::from_utf8(out).ok()
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(key: &str, has_extension: bool) -> Target {
        Target::Path {
            key: key.to_owned(),
            has_extension,
        }
    }

    #[test]
    fn plain_paths_become_keys() {
        assert_eq!(classify("/"), path("", false));
        assert_eq!(classify("/devices"), path("devices", false));
        assert_eq!(classify("/settings/x/"), path("settings/x", false));
        assert_eq!(classify("/assets/a-1.js"), path("assets/a-1.js", true));
        assert_eq!(classify("//assets//a.js"), path("assets/a.js", true));
    }

    #[test]
    fn api_prefixes_match_whole_segments_only() {
        for p in ["/api", "/api/x", "/admin/api", "/admin/api/v1/users"] {
            assert_eq!(classify(p), Target::Reject, "{p}");
        }
        assert_eq!(classify("/apix"), path("apix", false));
        assert_eq!(classify("/admin/apix"), path("admin/apix", false));
        assert_eq!(classify("/admin"), path("admin", false));
        assert_eq!(classify("/admin/settings"), path("admin/settings", false));
    }

    #[test]
    fn encoded_api_prefix_is_still_an_api_path() {
        assert_eq!(classify("/%61pi/unknown"), Target::Reject);
        assert_eq!(classify("/admin%2Fapi/x"), Target::Reject);
    }

    #[test]
    fn traversal_and_hidden_segments_are_rejected() {
        for p in [
            "/..",
            "/../etc/passwd",
            "/assets/../index.html",
            "/assets/%2e%2e/index.html",
            "/assets/%2E%2E%2findex.html",
            "/%2e%2e%5cindex.html",
            "/assets/..%2fx",
            "/./index.html",
            "/.env",
            "/assets/.hidden",
            "/a%00b",
            "/a\\b",
        ] {
            assert_eq!(classify(p), Target::Reject, "{p}");
        }
    }

    #[test]
    fn malformed_escapes_and_invalid_utf8_are_rejected() {
        for p in ["/%", "/%zz", "/%4", "/%ff", "/%c0%af"] {
            assert_eq!(classify(p), Target::Reject, "{p}");
        }
    }

    #[test]
    fn extension_is_judged_on_the_last_segment() {
        assert_eq!(classify("/v1.2/devices"), path("v1.2/devices", false));
        assert_eq!(classify("/devices/a.b"), path("devices/a.b", true));
    }
}
