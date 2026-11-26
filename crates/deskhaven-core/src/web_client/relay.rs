pub const RELAY_PATH: &str = "/ws/relay";

/// Only the CSP host-source characters, which `deskhaven_web::csp_connecting_to` accepts too.
pub fn ws_origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.trim().split_once("://")?;
    let ws = match scheme.to_ascii_lowercase().as_str() {
        "https" => "wss",
        "http" => "ws",
        _ => return None,
    };
    let authority = rest.split(['/', '?', '#']).next()?;
    let valid = !authority.is_empty()
        && authority
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-.:[]".contains(&b));
    valid.then(|| format!("{ws}://{}", authority.to_ascii_lowercase()))
}

pub fn relay_url(public_url: &str) -> String {
    match ws_origin(public_url) {
        Some(origin) => format!("{origin}{RELAY_PATH}"),
        None => RELAY_PATH.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_url_follows_the_public_url_scheme_and_host() {
        assert_eq!(
            relay_url("https://Desk.Example.com/console/"),
            "wss://desk.example.com/ws/relay"
        );
        assert_eq!(
            relay_url("http://192.0.2.1:21114"),
            "ws://192.0.2.1:21114/ws/relay"
        );
        assert_eq!(
            relay_url("https://[2001:db8::1]:8443"),
            "wss://[2001:db8::1]:8443/ws/relay"
        );
        assert_eq!(relay_url(""), "/ws/relay");
        assert_eq!(relay_url("ftp://desk.example.com"), "/ws/relay");
    }

    #[test]
    fn origins_refuse_what_an_authority_cannot_hold() {
        assert_eq!(ws_origin("https://a.example; script-src *"), None);
        assert_eq!(ws_origin("https://"), None);
        assert_eq!(ws_origin("https://user@a.example"), None);
        // A CSP host-source has no `_`: an origin the console CSP could not admit is none.
        assert_eq!(ws_origin("https://a_b.example"), None);
    }
}
