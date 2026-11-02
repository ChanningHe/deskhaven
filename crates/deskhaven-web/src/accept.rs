//! `Accept` handling for the SPA fallback: only requests that would take an
//! HTML document get `index.html`; `fetch()` of JSON or an `<img>` must not.

use axum::http::HeaderMap;
use axum::http::header::ACCEPT;

/// No `Accept` header means "anything". A malformed header value is not HTML.
pub(crate) fn accepts_html(headers: &HeaderMap) -> bool {
    let mut values = headers.get_all(ACCEPT).iter().peekable();
    if values.peek().is_none() {
        return true;
    }
    values
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(range_matches_html)
}

fn range_matches_html(range: &str) -> bool {
    let mut parts = range.split(';');
    let media = parts.next().unwrap_or_default().trim();
    let is_html = ["text/html", "text/*", "*/*"]
        .iter()
        .any(|m| media.eq_ignore_ascii_case(m));
    is_html && !parts.any(is_zero_quality)
}

/// `q=0`, `q=0.0`, `q=0.000`: explicitly not acceptable.
fn is_zero_quality(param: &str) -> bool {
    let Some((name, value)) = param.split_once('=') else {
        return false;
    };
    name.trim().eq_ignore_ascii_case("q") && value.trim().parse::<f32>().is_ok_and(|q| q == 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn accepts(value: Option<&str>) -> bool {
        let mut headers = HeaderMap::new();
        if let Some(v) = value {
            headers.insert(ACCEPT, HeaderValue::from_str(v).unwrap());
        }
        accepts_html(&headers)
    }

    #[test]
    fn missing_header_accepts_anything() {
        assert!(accepts(None));
    }

    #[test]
    fn browser_navigation_accepts_html() {
        let nav = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";
        assert!(accepts(Some(nav)));
        assert!(accepts(Some("TEXT/HTML")));
        assert!(accepts(Some("*/*")));
        assert!(accepts(Some("text/*;q=0.5")));
    }

    #[test]
    fn non_html_clients_are_refused() {
        assert!(!accepts(Some("application/json")));
        assert!(!accepts(Some("image/avif,image/webp,image/*;q=0.8")));
        assert!(!accepts(Some("text/html;q=0")));
        assert!(!accepts(Some("text/html; q=0.0, application/json")));
        assert!(!accepts(Some("")));
    }
}
