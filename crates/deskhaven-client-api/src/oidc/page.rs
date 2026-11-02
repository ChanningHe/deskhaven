//! The callback's HTML pages: no script, no style, nothing loaded (`default-src 'none'`),
//! forms only to this origin, never framed, cached or sent as a referrer (the URL carries
//! the code and state). Text from outside (the device's self-description, provider names)
//! is HTML-escaped; request parameters never appear (the form posts back to the page's own
//! URL, so the code and state are not copied into it).

use axum::http::{HeaderValue, header};
use axum::response::{Html, IntoResponse, Response};

const CSP: &str = "default-src 'none'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'";

/// A page saying `message` (already escaped or static).
pub(super) fn message(message: &str) -> Response {
    render(&format!("<p>{message}</p>"))
}

/// A question with one button that POSTs to this same URL (query included).
pub(super) fn confirm(question: &str, warning: &str, button: &str) -> Response {
    render(&format!(
        "<p>{question}</p><p>{warning}</p>\
         <form method=\"post\"><button type=\"submit\">{button}</button></form>"
    ))
}

fn render(body: &str) -> Response {
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width\">\
         <title>deskhaven</title></head><body>{body}</body></html>"
    );
    let mut response = Html(html).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// `text` safe inside HTML text and attribute values.
pub(super) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_neutralises_markup_and_control_characters() {
        assert_eq!(
            escape("<script>alert('x')</script> & \"q\"\n"),
            "&lt;script&gt;alert(&#x27;x&#x27;)&lt;/script&gt; &amp; &quot;q&quot;"
        );
    }
}
