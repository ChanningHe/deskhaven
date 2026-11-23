//! Content-Security-Policy for the console (06-security §6).

/// Set it as `HttpConfig::content_security_policy`
/// (`HeaderValue::from_static(CSP)`), or [`csp_connecting_to`] when the web client's relay
/// is on another origin than the console.
///
/// Derived from what the built SPA needs, nothing more:
/// - `default-src 'self'`: everything else is same-origin only.
/// - `script-src 'self'`: no inline script (the theme bootstrap is the external
///   `theme-init.js`), no `eval`.
/// - `style-src 'self'`: the build emits one external stylesheet; the app only
///   sets CSS variables through the CSSOM, which CSP allows. A style *element*
///   or `style=""` attribute would be blocked.
/// - `font-src 'self'`: the Geist woff2 files are served from `/assets/`.
/// - `img-src 'self' blob:`: `/favicon.svg` and the remote page's own cursor images; no `data:`.
/// - `connect-src 'self'`: the SPA talks to `/admin/api` and the web client's relay
///   (`/ws/relay`) on its own origin.
/// - `object-src 'none'`, `base-uri 'none'`: no plugins, no `<base>` hijack.
/// - `form-action 'self'`: forms can only post back to this origin.
/// - `frame-ancestors 'none'`: cannot be framed (clickjacking).
pub const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; \
font-src 'self'; img-src 'self' blob:; connect-src 'self'; object-src 'none'; \
base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

const CONNECT_SRC: &str = "connect-src 'self'";
const WS_SCHEMES: [&str; 2] = ["ws://", "wss://"];

/// [`CSP`] whose `connect-src` also admits the WebSocket origin `origin` (`ws://` or
/// `wss://` + host and port: the web client's relay on another origin than the console).
/// `None` for anything else, so nothing but an origin can reach the header.
pub fn csp_connecting_to(origin: &str) -> Option<String> {
    let authority = WS_SCHEMES
        .iter()
        .find_map(|scheme| origin.strip_prefix(scheme))?;
    let valid = !authority.is_empty()
        && authority
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-.:[]".contains(&b));
    valid.then(|| CSP.replace(CONNECT_SRC, &format!("{CONNECT_SRC} {origin}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relay_origin_joins_connect_src_only() {
        let csp = csp_connecting_to("wss://relay.example.com:8443").unwrap();

        assert_eq!(
            csp,
            CSP.replace(
                "connect-src 'self';",
                "connect-src 'self' wss://relay.example.com:8443;"
            )
        );
        assert_eq!(
            csp_connecting_to("ws://[2001:db8::1]:21114").unwrap(),
            CSP.replace(
                "connect-src 'self';",
                "connect-src 'self' ws://[2001:db8::1]:21114;"
            )
        );
    }

    #[test]
    fn images_admit_blob_urls_besides_self_and_nothing_else() {
        let img_src = |csp: &str| {
            csp.split(';')
                .map(str::trim)
                .find(|d| d.starts_with("img-src"))
                .map(str::to_owned)
        };
        let relayed = csp_connecting_to("wss://relay.example.com:8443").unwrap();

        assert_eq!(img_src(CSP).as_deref(), Some("img-src 'self' blob:"));
        assert_eq!(img_src(&relayed).as_deref(), Some("img-src 'self' blob:"));
        assert!(!CSP.contains("data:"));
    }

    #[test]
    fn anything_but_a_websocket_origin_is_refused() {
        for bad in [
            "",
            "wss://",
            "https://relay.example.com",
            "wss://relay.example.com/ws/relay",
            "wss://a.example; script-src *",
            "wss://*",
            "wss://user@a.example",
        ] {
            assert_eq!(csp_connecting_to(bad), None, "{bad:?}");
        }
    }
}
