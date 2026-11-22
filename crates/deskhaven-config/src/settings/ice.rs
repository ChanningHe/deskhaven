//! ICE server URLs (RFC 7064 `stun:` / `stuns:`, RFC 7065 `turn:` / `turns:`) as the web
//! client setting lists them: comma separated, each `scheme:host[:port]`, TURN optionally
//! with `?transport=udp|tcp`. Credentials are separate settings, never part of a URL.

use super::value::is_host;

/// Most ICE servers the web client hands a browser.
pub const MAX_ICE_SERVERS: usize = 8;

const STUN_SCHEMES: [&str; 2] = ["stun", "stuns"];
const TURN_SCHEMES: [&str; 2] = ["turn", "turns"];
const TRANSPORTS: [&str; 2] = ["transport=udp", "transport=tcp"];

/// The URLs of an ICE server list (blank items are not URLs: the list check refuses them).
pub fn ice_urls(text: &str) -> impl Iterator<Item = &str> {
    text.split(',').map(str::trim).filter(|url| !url.is_empty())
}

/// Whether `url` is a TURN URL (one the TURN credentials apply to).
pub fn is_turn_url(url: &str) -> bool {
    url.split_once(':')
        .is_some_and(|(scheme, _)| TURN_SCHEMES.contains(&scheme))
}

/// The lowercased host of a valid TURN URL (IPv6 without brackets); `None` for other URLs.
pub(crate) fn turn_host(url: &str) -> Option<String> {
    let authority = url.split_once(':')?.1.split('?').next()?;
    let host = match authority.strip_prefix('[') {
        Some(rest) => rest.split(']').next()?,
        None => authority.split(':').next()?,
    };
    is_turn_url(url).then(|| host.to_ascii_lowercase())
}

/// A non-empty list of at most [`MAX_ICE_SERVERS`] valid URLs, no blank items.
pub(crate) fn is_ice_url_list(text: &str) -> bool {
    let items: Vec<&str> = text.split(',').map(str::trim).collect();
    items.len() <= MAX_ICE_SERVERS && items.iter().all(|url| is_ice_url(url))
}

fn is_ice_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once(':') else {
        return false;
    };
    let turn = TURN_SCHEMES.contains(&scheme);
    if !turn && !STUN_SCHEMES.contains(&scheme) {
        return false;
    }
    let (authority, query) = match rest.split_once('?') {
        Some((authority, query)) => (authority, Some(query)),
        None => (rest, None),
    };
    let query_ok = match query {
        None => true,
        Some(query) => turn && TRANSPORTS.contains(&query),
    };
    query_ok && is_authority(authority)
}

/// `host`, `host:port`, `[v6]` or `[v6]:port`; port 1-65535.
fn is_authority(authority: &str) -> bool {
    if let Some(rest) = authority.strip_prefix('[') {
        let Some((host, port)) = rest.split_once(']') else {
            return false;
        };
        let port_ok = port.is_empty() || port.strip_prefix(':').is_some_and(is_port);
        return port_ok && host.contains(':') && is_host(host);
    }
    match authority.split_once(':') {
        Some((host, port)) => is_port(port) && is_host(host),
        None => is_host(authority),
    }
}

fn is_port(port: &str) -> bool {
    !port.is_empty()
        && port.bytes().all(|b| b.is_ascii_digit())
        && port.parse::<u16>().is_ok_and(|p| p != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_and_classifies_urls() {
        let urls: Vec<&str> = ice_urls(" stun:a.example , turns:b.example:5349 ").collect();
        assert_eq!(urls, ["stun:a.example", "turns:b.example:5349"]);
        assert!(is_turn_url("turns:b.example:5349"));
        assert!(!is_turn_url("stun:a.example"));
        assert!(!is_turn_url("turnx:a.example"));
    }

    #[test]
    fn authorities_need_a_host_and_a_valid_port() {
        for good in [
            "a.example",
            "a.example:1",
            "192.0.2.1:3478",
            "[::1]",
            "[::1]:3478",
        ] {
            assert!(is_authority(good), "{good}");
        }
        for bad in [
            "",
            ":3478",
            "a.example:",
            "a.example:65536",
            "[::1",
            "[a.example]",
            "::1",
        ] {
            assert!(!is_authority(bad), "{bad}");
        }
    }
}
