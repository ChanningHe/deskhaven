//! Text constraints: length, forbidden characters, optional format.
//! The empty string always passes the format check (it means "not set").

use serde_json::{Map, Value as Json};

use super::value::is_host;

/// Shape a non-empty text value must have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Any,
    /// DNS name, IPv4 or IPv6 (unbracketed, no port).
    Host,
    /// `local@domain`: one `@`, both sides non-empty, no whitespace.
    Email,
}

/// Text constraints; control and invisible formatting characters are
/// always rejected.
#[derive(Debug, Clone, Copy)]
pub struct TextRule {
    max_len: usize,
    format: Format,
}

pub const fn max_len(max_len: usize) -> TextRule {
    TextRule {
        max_len,
        format: Format::Any,
    }
}

impl TextRule {
    pub const fn host(self) -> Self {
        Self {
            format: Format::Host,
            ..self
        }
    }

    pub const fn email(self) -> Self {
        Self {
            format: Format::Email,
            ..self
        }
    }

    pub(crate) fn check(&self, text: &str) -> Result<(), String> {
        if text.chars().count() > self.max_len {
            return Err(format!("must be at most {} characters", self.max_len));
        }
        if text
            .chars()
            .any(|c| c.is_control() || is_invisible_format(c))
        {
            return Err("must not contain control or invisible formatting characters".to_owned());
        }
        match self.format {
            _ if text.is_empty() => Ok(()),
            Format::Any => Ok(()),
            Format::Host if is_host(text) => Ok(()),
            Format::Host => Err("must be a host name or IP address".to_owned()),
            Format::Email if is_email(text) => Ok(()),
            Format::Email => Err("must be an email address (local@domain)".to_owned()),
        }
    }

    pub(crate) fn describe(&self) -> Map<String, Json> {
        let mut map = Map::from_iter([("max_length".to_owned(), Json::from(self.max_len))]);
        let format = match self.format {
            Format::Any => None,
            Format::Host => Some("host"),
            Format::Email => Some("email"),
        };
        if let Some(format) = format {
            map.insert("format".to_owned(), Json::from(format));
        }
        map
    }
}

fn is_email(text: &str) -> bool {
    match text.split_once('@') {
        Some((local, domain)) => {
            !local.is_empty()
                && !domain.is_empty()
                && !domain.contains('@')
                && !text.chars().any(char::is_whitespace)
        }
        None => false,
    }
}

/// Zero-width, bidi-control and line/paragraph separator characters
/// (Unicode `Cf`/`Zl`/`Zp` members that render invisibly or reorder text).
fn is_invisible_format(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{2028}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_control_invisible_and_overlong() {
        assert!(max_len(2).check("ok").is_ok());
        assert!(max_len(2).check("long").is_err());
        assert!(max_len(9).check("a\nb").is_err());
        for invisible in ["a\u{202E}b", "a\u{200B}b", "a\u{2028}b", "\u{FEFF}a"] {
            assert!(max_len(9).check(invisible).is_err(), "{invisible:?}");
        }
        assert!(max_len(20).check("Ünïcode ok").is_ok());
    }

    #[test]
    fn formats_allow_empty_and_check_shape() {
        let host = max_len(254).host();
        let email = max_len(254).email();
        assert!(host.check("").is_ok() && email.check("").is_ok());
        assert!(host.check("::1").is_ok());
        assert!(host.check("[::1]").is_err());
        assert!(email.check("a@b").is_ok());
        assert!(email.check("a\t@b").is_err());
    }
}
