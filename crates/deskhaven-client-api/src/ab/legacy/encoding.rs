//! Content negotiation of the legacy pull: the client asks with `Accept-Encoding: gzip`
//! (`flutter/lib/models/ab_model.dart:1015`) and its HTTP stack inflates a gzip reply by
//! itself. Pushes are never compressed (since 2026-01 the client does not; before, only when
//! `licensed_devices` > 0, which deskhaven never sends).

use std::io::Write;

use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use flate2::Compression;
use flate2::write::GzEncoder;

use crate::ClientApiError;

const GZIP: &str = "gzip";
/// The older spelling of the same coding (RFC 9110 §8.4.1.3).
const X_GZIP: &str = "x-gzip";
const IDENTITY: &str = "identity";
const JSON: &str = "application/json";

/// Whether `Accept-Encoding` allows a gzip body: a `gzip` (or `x-gzip`) member whose
/// weight is not zero. `*` is not read as gzip: callers that accept anything also accept
/// plain bytes.
pub fn accepts_gzip(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::ACCEPT_ENCODING)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|member| {
            let mut parts = member.split(';');
            let coding = parts.next().unwrap_or_default().trim();
            let named = coding.eq_ignore_ascii_case(GZIP) || coding.eq_ignore_ascii_case(X_GZIP);
            named && !parts.any(is_zero_weight)
        })
}

/// `q=0`, `q=0.0`, ... (any spelling of zero).
fn is_zero_weight(param: &str) -> bool {
    let Some((name, weight)) = param.split_once('=') else {
        return false;
    };
    name.trim().eq_ignore_ascii_case("q") && weight.trim().parse::<f32>().is_ok_and(|w| w == 0.0)
}

/// Whether a request body may be read as plain JSON: no `Content-Encoding`, or `identity`.
pub fn is_plain(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::CONTENT_ENCODING)
        .iter()
        .all(|value| {
            value
                .to_str()
                .is_ok_and(|v| v.trim().eq_ignore_ascii_case(IDENTITY))
        })
}

/// A `200` JSON reply, gzipped when `gzip`. `Vary` always names the header the choice
/// depends on, so a cache never hands a compressed body to a client that did not ask.
/// Compressing a large book takes long enough to stall an async worker, so it runs on the
/// blocking pool.
pub async fn json_reply(body: String, gzip: bool) -> Result<Response, ClientApiError> {
    let mut response = if gzip {
        let compressed = tokio::task::spawn_blocking(move || compress(body.as_bytes()))
            .await
            .map_err(ClientApiError::internal)?
            .map_err(ClientApiError::internal)?;
        let mut response = (StatusCode::OK, compressed).into_response();
        response
            .headers_mut()
            .insert(header::CONTENT_ENCODING, HeaderValue::from_static(GZIP));
        response
    } else {
        (StatusCode::OK, body).into_response()
    };
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(JSON));
    headers.insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    Ok(response)
}

fn compress(plain: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(plain)?;
    encoder.finish()
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use flate2::read::GzDecoder;

    use super::*;

    fn accept(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ACCEPT_ENCODING,
            HeaderValue::from_str(value).unwrap(),
        );
        headers
    }

    #[test]
    fn gzip_is_accepted_by_name_unless_its_weight_is_zero() {
        for yes in [
            "gzip",
            "GZIP",
            " gzip ",
            "br, gzip;q=0.1",
            "gzip; q=1",
            "x-gzip",
            "identity,gzip",
        ] {
            assert!(accepts_gzip(&accept(yes)), "{yes}");
        }
        for no in [
            "",
            "identity",
            "*",
            "gzip;q=0",
            "gzip; Q=0.0",
            "gzipped",
            "deflate, br",
        ] {
            assert!(!accepts_gzip(&accept(no)), "{no}");
        }
        assert!(!accepts_gzip(&HeaderMap::new()));
    }

    #[test]
    fn repeated_headers_are_all_read() {
        let mut headers = accept("br");
        headers.append(header::ACCEPT_ENCODING, HeaderValue::from_static("gzip"));

        assert!(accepts_gzip(&headers));
    }

    #[test]
    fn a_request_body_is_plain_without_an_encoding_or_with_identity() {
        let mut headers = HeaderMap::new();
        assert!(is_plain(&headers));
        headers.insert(
            header::CONTENT_ENCODING,
            HeaderValue::from_static("Identity"),
        );
        assert!(is_plain(&headers));
        headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        assert!(!is_plain(&headers));
    }

    #[tokio::test]
    async fn replies_are_labelled_and_the_gzip_one_inflates_to_the_body() {
        let plain = json_reply("[1,2]".to_owned(), false).await.unwrap();
        let zipped = json_reply("[1,2]".to_owned(), true).await.unwrap();

        assert!(plain.headers().get(header::CONTENT_ENCODING).is_none());
        assert_eq!(zipped.headers()[header::CONTENT_ENCODING], "gzip");
        for reply in [&plain, &zipped] {
            assert_eq!(reply.headers()[header::VARY], "Accept-Encoding");
            assert_eq!(reply.headers()[header::CONTENT_TYPE], JSON);
        }
        let (_, body) = zipped.into_parts();
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        let mut inflated = String::new();
        GzDecoder::new(&bytes[..])
            .read_to_string(&mut inflated)
            .unwrap();
        assert_eq!(inflated, "[1,2]");
    }
}
