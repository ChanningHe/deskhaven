//! The option map of a strategy: client option name to value (05 §4.3).
//!
//! An empty value is meaningful: it makes the client delete that option (03 §2.1), so it is
//! valid input and is kept through storage and merging.

use std::collections::BTreeMap;

use super::StrategyError;
use crate::text;

/// `config_options` as sent to the client.
pub type Options = BTreeMap<String, String>;

/// Changes to single options: `Some(value)` sets the option (an empty value included),
/// `None` removes it from the strategy. Options not named stay as they are.
pub type OptionsPatch = BTreeMap<String, Option<String>>;

/// `options` with `patch` applied.
pub(super) fn patched(mut options: Options, patch: &OptionsPatch) -> Options {
    for (key, change) in patch {
        match change {
            Some(value) => drop(options.insert(key.clone(), value.clone())),
            None => drop(options.remove(key)),
        }
    }
    options
}

/// The IP whitelist option: comma-separated IP addresses or CIDR ranges.
const IP_WHITELIST: &str = "whitelist";

/// `options` as clients should get them: the IP whitelist without spaces around its entries
/// and without empty entries. Clients trim entries since rustdesk 78450f4 (#16692,
/// connection.rs `ip_whitelist_entries`); older ones do not, so `"10.0.0.0/8, 192.0.2.1"`
/// blocks 192.0.2.1 on them. Applied on write and again when loading what devices get, which
/// also covers rows stored before.
pub(super) fn normalized(options: Options) -> Options {
    options
        .into_iter()
        .map(|(key, value)| match key.as_str() {
            IP_WHITELIST => {
                let entries: Vec<&str> = value
                    .split(',')
                    .map(str::trim)
                    .filter(|entry| !entry.is_empty())
                    .collect();
                (key, entries.join(","))
            }
            _ => (key, value),
        })
        .collect()
}

/// Most options one strategy may carry.
pub const MAX_OPTIONS: usize = 256;
/// Longest option name, in characters.
pub const KEY_MAX_LEN: usize = 128;
/// Longest option value, in characters (whitelists and server lists can be long).
pub const VALUE_MAX_LEN: usize = 4096;

fn invalid(reason: String) -> StrategyError {
    StrategyError::Invalid {
        field: "options",
        reason,
    }
}

/// Checks count and sizes; names are non-empty without whitespace or control characters,
/// values may be empty. Values are never echoed in the error.
pub(super) fn validate(options: &Options) -> Result<(), StrategyError> {
    if options.len() > MAX_OPTIONS {
        return Err(invalid(format!("at most {MAX_OPTIONS} options")));
    }
    for (key, value) in options {
        if key.is_empty() {
            return Err(invalid("option names must not be empty".to_owned()));
        }
        if key.chars().any(char::is_whitespace) {
            return Err(invalid(format!("option name {key:?} contains whitespace")));
        }
        if let Some(reason) = text::problem(key, KEY_MAX_LEN, false) {
            return Err(invalid(format!("option name {key:?} {reason}")));
        }
        if let Some(reason) = text::problem(value, VALUE_MAX_LEN, false) {
            return Err(invalid(format!("value of option {key:?} {reason}")));
        }
    }
    Ok(())
}

/// The stored form: a JSON object with sorted keys.
pub(super) fn encode(options: &Options) -> Result<String, StrategyError> {
    serde_json::to_string(options).map_err(|e| invalid(e.to_string()))
}

/// `None` when `stored` is not a JSON object of strings.
pub(super) fn decode(stored: &str) -> Option<Options> {
    serde_json::from_str(stored).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(key: &str, value: &str) -> Options {
        Options::from([(key.to_owned(), value.to_owned())])
    }

    #[test]
    fn accepts_empty_values_and_rejects_bad_names_and_sizes() {
        assert!(validate(&one("enable-file-transfer", "")).is_ok());
        assert!(validate(&Options::new()).is_ok());
        for bad in [
            one("", "v"),
            one("has space", "v"),
            one("tab\t", "v"),
            one(&"k".repeat(KEY_MAX_LEN + 1), "v"),
            one("k", &"v".repeat(VALUE_MAX_LEN + 1)),
            one("k", "line\nbreak"),
        ] {
            assert!(
                matches!(validate(&bad), Err(StrategyError::Invalid { .. })),
                "{bad:?}"
            );
        }
        let edge = one(&"k".repeat(KEY_MAX_LEN), &"v".repeat(VALUE_MAX_LEN));
        assert!(validate(&edge).is_ok());
        let many: Options = (0..=MAX_OPTIONS)
            .map(|i| (format!("k{i}"), String::new()))
            .collect();
        assert!(validate(&many).is_err());
    }

    #[test]
    fn ip_whitelist_entries_lose_surrounding_spaces_and_empty_entries() {
        let cases = [
            ("10.0.0.0/8, 192.0.2.1", "10.0.0.0/8,192.0.2.1"),
            (" 10.0.0.1 ,\t2001:db8::/32 ", "10.0.0.1,2001:db8::/32"),
            ("10.0.0.1,,192.0.2.1,", "10.0.0.1,192.0.2.1"),
            ("10.0.0.1,192.0.2.1", "10.0.0.1,192.0.2.1"),
            ("0.0.0.0", "0.0.0.0"),
            (" , ", ""),
            ("", ""),
        ];
        for (value, want) in cases {
            assert_eq!(
                normalized(one("whitelist", value)),
                one("whitelist", want),
                "{value:?}"
            );
        }
    }

    #[test]
    fn other_options_are_left_as_they_are() {
        let options = Options::from([
            ("id-whitelist".to_owned(), "123, 456".to_owned()),
            ("custom-rendezvous-server".to_owned(), " host ".to_owned()),
            ("whitelist".to_owned(), "10.0.0.1, 10.0.0.2".to_owned()),
        ]);

        let got = normalized(options.clone());

        assert_eq!(got["id-whitelist"], "123, 456");
        assert_eq!(got["custom-rendezvous-server"], " host ");
        assert_eq!(got["whitelist"], "10.0.0.1,10.0.0.2");
        assert_eq!(got.len(), options.len());
    }

    #[test]
    fn stored_form_round_trips_and_rejects_other_json() {
        let options = one("a", "");
        assert_eq!(decode(&encode(&options).unwrap()), Some(options));
        for stored in ["[]", "{\"a\":1}", "not json", "null"] {
            assert_eq!(decode(stored), None, "{stored}");
        }
    }
}
