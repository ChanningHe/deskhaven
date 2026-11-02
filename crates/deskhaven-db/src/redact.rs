//! Credential redaction for database URLs (logs, `Debug`, errors).

const MASK: &str = "***";

/// Masks the userinfo password and any `*password*` query parameter.
/// `sqlite:` URLs carry no credentials and are returned unchanged.
pub(crate) fn redact_url(url: &str) -> String {
    if url.starts_with("sqlite:") {
        return url.to_owned();
    }
    let (base, query) = match url.split_once('?') {
        Some((base, query)) => (base, Some(query)),
        None => (url, None),
    };
    let mut out = redact_userinfo(base);
    if let Some(query) = query {
        out.push('?');
        out.push_str(&redact_query(query));
    }
    out
}

/// Uses the last `@` so a raw `@` or `/` inside the password still ends up masked.
fn redact_userinfo(base: &str) -> String {
    let Some((scheme, rest)) = base.split_once("://") else {
        return base.to_owned();
    };
    let Some(at) = rest.rfind('@') else {
        return base.to_owned();
    };
    let (userinfo, host) = rest.split_at(at);
    match userinfo.split_once(':') {
        Some((user, _)) => format!("{scheme}://{user}:{MASK}{host}"),
        None => base.to_owned(),
    }
}

fn redact_query(query: &str) -> String {
    query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((key, _)) if key.to_ascii_lowercase().contains("password") => {
                format!("{key}={MASK}")
            }
            _ => pair.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("&")
}

#[cfg(test)]
mod tests {
    use super::redact_url;

    #[test]
    fn masks_userinfo_password() {
        assert_eq!(
            redact_url("postgres://u:hunter2@h:5432/db?sslmode=require"),
            "postgres://u:***@h:5432/db?sslmode=require"
        );
    }

    #[test]
    fn masks_password_with_raw_at_and_slash() {
        assert_eq!(
            redact_url("postgres://u:p@s/s@h/db"),
            "postgres://u:***@h/db"
        );
    }

    #[test]
    fn masks_password_query_params() {
        assert_eq!(
            redact_url("postgresql://h/db?password=hunter2&sslmode=x&sslpassword=k"),
            "postgresql://h/db?password=***&sslmode=x&sslpassword=***"
        );
    }

    #[test]
    fn keeps_urls_without_secrets() {
        for url in [
            "postgres://u@h/db",
            "postgres://h/db",
            "sqlite:///tmp/a@b.db",
        ] {
            assert_eq!(redact_url(url), url);
        }
    }
}
