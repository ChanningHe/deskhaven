/// Derived from what the built SPA needs: no inline script (the theme bootstrap is
/// `theme-init.js`) or `eval`; one external stylesheet (CSS variables go through the CSSOM,
/// which CSP allows); `blob:` images for the remote page's cursors, no `data:`;
/// `connect-src 'self'` covers `/admin/api` and the relay at `/ws/relay`. Use
/// [`csp_connecting_to`] when the relay is on another origin.
pub const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; \
font-src 'self'; img-src 'self' blob:; connect-src 'self'; object-src 'none'; \
base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

const CONNECT_SRC: &str = "connect-src 'self'";
const WS_SCHEMES: [&str; 2] = ["ws://", "wss://"];

/// `None` for anything but a `ws://` / `wss://` origin, so nothing else can reach the header.
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
