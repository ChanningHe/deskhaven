pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Where the corresponding source of this build is offered. A constant on
/// purpose: a deployment must not be able to point its users elsewhere by
/// configuration; a fork changes it here when it changes the code.
pub const SOURCE_URL: &str = "https://github.com/ChanningHe/deskhaven";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_url_is_an_absolute_https_url_without_trailing_slash() {
        assert!(SOURCE_URL.starts_with("https://"));
        assert!(!SOURCE_URL.ends_with('/'));
    }
}
