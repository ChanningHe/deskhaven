//! Helpers shared by the API error types.

use std::error::Error;
use std::fmt::Write;

/// `outer: cause: root cause` — the full source chain for server-side logs.
pub fn error_chain(err: &(dyn Error + 'static)) -> String {
    let mut out = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        // Writing to a String cannot fail.
        let _ = write!(out, ": {cause}");
        source = cause.source();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, thiserror::Error)]
    #[error("outer")]
    struct Outer(#[source] std::io::Error);

    #[test]
    fn joins_the_source_chain() {
        let err = Outer(std::io::Error::other("disk on fire"));
        assert_eq!(error_chain(&err), "outer: disk on fire");
    }

    #[test]
    fn single_error_is_just_its_message() {
        assert_eq!(error_chain(&std::io::Error::other("x")), "x");
    }
}
