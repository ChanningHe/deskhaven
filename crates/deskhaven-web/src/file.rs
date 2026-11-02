//! One embedded file as an HTTP response: Content-Type, strong ETag,
//! `If-None-Match` -> 304, Cache-Control.

use std::borrow::Cow;
use std::fmt::Write as _;

use axum::body::Body;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, ETAG, IF_NONE_MATCH};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use rust_embed::EmbeddedFile;

/// Vite fingerprints everything it emits under `assets/`.
const HASHED_DIR: &str = "assets/";
const IMMUTABLE: HeaderValue = HeaderValue::from_static("public, max-age=31536000, immutable");
const REVALIDATE: HeaderValue = HeaderValue::from_static("no-cache");
/// First 16 bytes of the SHA-256 are plenty for a validator.
const ETAG_HASH_BYTES: usize = 16;

pub(crate) struct File {
    data: Cow<'static, [u8]>,
    content_type: String,
    etag: String,
    cache_control: HeaderValue,
}

impl File {
    pub(crate) fn new(key: &str, embedded: EmbeddedFile) -> Self {
        let cache_control = if key.starts_with(HASHED_DIR) {
            IMMUTABLE
        } else {
            REVALIDATE
        };
        Self {
            content_type: content_type(embedded.metadata.mimetype()),
            etag: etag(&embedded.metadata.sha256_hash()),
            cache_control,
            data: embedded.data,
        }
    }

    /// 200 with the body (empty for HEAD), or 304 when the client's cache is current.
    pub(crate) fn respond(self, request_headers: &HeaderMap) -> Response {
        let Ok(etag) = HeaderValue::from_str(&self.etag) else {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        };
        let Ok(content_type) = HeaderValue::from_str(&self.content_type) else {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        };
        let headers = [
            (CONTENT_TYPE, content_type),
            (ETAG, etag),
            (CACHE_CONTROL, self.cache_control),
        ];
        if matches_etag(request_headers, &self.etag) {
            // A 304 carries the validator and caching headers, not the entity ones.
            let [_, etag, cache] = headers;
            return (StatusCode::NOT_MODIFIED, [etag, cache]).into_response();
        }
        // HEAD gets the full body too: axum strips it after fixing Content-Length,
        // so the length matches GET.
        (StatusCode::OK, headers, Body::from(self.data)).into_response()
    }
}

/// Text types are always UTF-8 (Vite emits UTF-8).
fn content_type(mimetype: &str) -> String {
    if mimetype.starts_with("text/") {
        format!("{mimetype}; charset=utf-8")
    } else {
        mimetype.to_owned()
    }
}

fn etag(hash: &[u8]) -> String {
    let mut tag = String::with_capacity(ETAG_HASH_BYTES * 2 + 2);
    tag.push('"');
    for byte in hash.iter().take(ETAG_HASH_BYTES) {
        // Writing to a String cannot fail.
        let _ = write!(tag, "{byte:02x}");
    }
    tag.push('"');
    tag
}

/// RFC 9110 `If-None-Match`: `*` or any listed tag, weak comparison.
fn matches_etag(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get_all(IF_NONE_MATCH)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .any(|candidate| {
            candidate == "*" || candidate.strip_prefix("W/").unwrap_or(candidate) == etag
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn if_none_match(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(IF_NONE_MATCH, HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn etag_is_quoted_lowercase_hex_of_the_hash_prefix() {
        let hash: Vec<u8> = (0..32).collect();
        assert_eq!(etag(&hash), "\"000102030405060708090a0b0c0d0e0f\"");
    }

    #[test]
    fn text_types_get_a_charset_and_binary_types_do_not() {
        assert_eq!(content_type("text/css"), "text/css; charset=utf-8");
        assert_eq!(content_type("font/woff2"), "font/woff2");
    }

    #[test]
    fn if_none_match_accepts_lists_weak_tags_and_star() {
        let tag = "\"abc\"";
        assert!(matches_etag(&if_none_match("\"abc\""), tag));
        assert!(matches_etag(&if_none_match("\"x\", \"abc\""), tag));
        assert!(matches_etag(&if_none_match("W/\"abc\""), tag));
        assert!(matches_etag(&if_none_match("*"), tag));
        assert!(!matches_etag(&if_none_match("\"abd\""), tag));
        // Only one weak marker is valid syntax.
        assert!(!matches_etag(&if_none_match("W/W/\"abc\""), tag));
        assert!(!matches_etag(&HeaderMap::new(), tag));
    }
}
