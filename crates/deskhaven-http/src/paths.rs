//! URL prefixes the stack needs to know about.

/// RustDesk client API (03).
pub const CLIENT_API_PREFIX: &str = "/api";
/// Admin API (08 §6).
pub const ADMIN_API_PREFIX: &str = "/admin/api";

/// `path` is `prefix` itself or below it (`/api`, `/api/x`, not `/apix`).
fn is_under(path: &str, prefix: &str) -> bool {
    path.strip_prefix(prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

pub(crate) fn is_client_api(path: &str) -> bool {
    is_under(path, CLIENT_API_PREFIX)
}

pub(crate) fn is_api(path: &str) -> bool {
    is_client_api(path) || is_under(path, ADMIN_API_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_match_whole_segments() {
        assert!(is_client_api("/api"));
        assert!(is_client_api("/api/heartbeat"));
        assert!(!is_client_api("/apix"));
        assert!(!is_client_api("/admin/api/v1"));
        assert!(is_api("/admin/api"));
        assert!(is_api("/admin/api/v1/users"));
        assert!(!is_api("/admin/apix"));
        assert!(!is_api("/assets/app.js"));
    }
}
