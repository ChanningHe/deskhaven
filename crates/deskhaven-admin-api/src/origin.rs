//! The browser-side half of the CSRF defence; the other half is the `X-Deskhaven-CSRF` header,
//! which still applies to a request with neither `Sec-Fetch-Site` nor `Origin`.

use std::sync::Arc;

use axum::http::{HeaderMap, header};

use crate::AdminApiError;

const SEC_FETCH_SITE: &str = "sec-fetch-site";
const ALLOWED_FETCH_SITES: [&str; 2] = ["same-origin", "none"];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrustedOrigins(Arc<[String]>);

impl TrustedOrigins {
    pub fn from_urls<'a>(urls: impl IntoIterator<Item = &'a str>) -> Self {
        Self(urls.into_iter().filter_map(url_origin).collect())
    }

    fn contains(&self, origin: &str) -> bool {
        self.0.iter().any(|o| o == origin)
    }

    pub(crate) fn any_https(&self) -> bool {
        self.0.iter().any(|o| o.starts_with("https://"))
    }
}

pub(crate) fn check_same_origin(
    headers: &HeaderMap,
    trusted: &TrustedOrigins,
) -> Result<(), AdminApiError> {
    if let Some(site) = headers.get(SEC_FETCH_SITE) {
        let site = site.to_str().unwrap_or_default();
        return if ALLOWED_FETCH_SITES.contains(&site) {
            Ok(())
        } else {
            Err(cross_site())
        };
    }
    let Some(origin) = headers.get(header::ORIGIN) else {
        return Ok(());
    };
    let origin = origin
        .to_str()
        .ok()
        .and_then(header_origin)
        .ok_or_else(cross_site)?;
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(str::to_ascii_lowercase);
    let authority = origin.split_once("://").map(|(_, a)| a);
    if trusted.contains(&origin) || (host.is_some() && authority == host.as_deref()) {
        Ok(())
    } else {
        Err(cross_site())
    }
}

fn cross_site() -> AdminApiError {
    AdminApiError::new(crate::ErrorCode::CsrfFailed, "cross-site request refused")
}

fn url_origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.trim().split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    origin_of(scheme, authority)
}

fn header_origin(value: &str) -> Option<String> {
    let (scheme, authority) = value.split_once("://")?;
    if authority.contains(['/', '?', '#']) {
        return None;
    }
    origin_of(scheme, authority)
}

fn origin_of(scheme: &str, authority: &str) -> Option<String> {
    let valid = !scheme.is_empty()
        && !authority.is_empty()
        && scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b));
    valid.then(|| format!("{scheme}://{authority}").to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    fn trusted() -> TrustedOrigins {
        TrustedOrigins::from_urls(["https://Console.Example.com/admin/", "", "nonsense"])
    }

    #[test]
    fn configured_urls_reduce_to_origins() {
        assert_eq!(
            trusted(),
            TrustedOrigins(vec!["https://console.example.com".to_owned()].into())
        );
    }

    #[test]
    fn fetch_metadata_decides_when_present() {
        for site in ["same-origin", "none"] {
            let h = headers(&[("sec-fetch-site", site), ("origin", "https://evil.test")]);
            assert!(check_same_origin(&h, &trusted()).is_ok(), "{site}");
        }
        for site in ["cross-site", "same-site", ""] {
            let h = headers(&[("sec-fetch-site", site)]);
            assert!(check_same_origin(&h, &trusted()).is_err(), "{site}");
        }
    }

    #[test]
    fn origin_must_be_trusted_or_match_host() {
        let ok = [
            headers(&[("origin", "https://console.example.com")]),
            headers(&[
                ("origin", "http://rd.local:8080"),
                ("host", "RD.local:8080"),
            ]),
            headers(&[]),
        ];
        for h in ok {
            assert!(check_same_origin(&h, &trusted()).is_ok(), "{h:?}");
        }
        let refused = [
            headers(&[("origin", "https://evil.test"), ("host", "rd.local")]),
            headers(&[("origin", "null"), ("host", "rd.local")]),
            headers(&[("origin", "https://rd.local/x"), ("host", "rd.local")]),
            headers(&[("origin", "https://evil.test")]),
        ];
        for h in refused {
            assert!(check_same_origin(&h, &trusted()).is_err(), "{h:?}");
        }
    }
}
