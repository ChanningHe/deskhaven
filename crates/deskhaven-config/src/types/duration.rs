use std::fmt;
use std::str::FromStr;

/// A duration written with explicit units, e.g. `"15m"`, `"7d"`, `"1h30m"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Duration(std::time::Duration);

impl Duration {
    pub const fn from_std(d: std::time::Duration) -> Self {
        Self(d)
    }

    pub const fn as_std(&self) -> std::time::Duration {
        self.0
    }
}

impl From<Duration> for std::time::Duration {
    fn from(d: Duration) -> Self {
        d.0
    }
}

/// Parse failure for [`Duration`]. Never includes the input.
#[derive(Debug, thiserror::Error)]
#[error("invalid duration: expected a number with units, e.g. \"15m\", \"7d\", \"1h30m\"")]
pub struct DurationError;

impl FromStr for Duration {
    type Err = DurationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        humantime::parse_duration(s.trim())
            .map(Self)
            .map_err(|_| DurationError)
    }
}

/// Whole units from days down to seconds; humantime's own formatter uses
/// fractional months/years (`180d` -> `5months 27days 22h 30m 36s`).
const UNITS: [(&str, u64); 4] = [("d", 86_400), ("h", 3_600), ("m", 60), ("s", 1)];
const SUBSEC_UNITS: [(&str, u32); 3] = [("ms", 1_000_000), ("us", 1_000), ("ns", 1)];

/// Exact and compact (`"30d"`, `"1h30m"`, `"1m"`, `"0s"`); parses back to the same value.
impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_zero() {
            return f.write_str("0s");
        }
        let mut secs = self.0.as_secs();
        for (unit, size) in UNITS {
            if secs >= size {
                write!(f, "{}{unit}", secs / size)?;
                secs %= size;
            }
        }
        let nanos = self.0.subsec_nanos();
        match SUBSEC_UNITS
            .iter()
            .find(|(_, size)| nanos.is_multiple_of(*size))
        {
            Some((unit, size)) if nanos > 0 => write!(f, "{}{unit}", nanos / size),
            _ => Ok(()),
        }
    }
}

super::string_serde!(
    Duration,
    "a duration string with units, e.g. \"15m\" or \"7d\""
);
super::string_schema!(
    Duration,
    "Duration",
    "Duration with units (humantime)",
    "15m"
);

#[cfg(test)]
mod tests {
    use super::*;
    use figment::Figment;
    use figment::providers::{Format, Toml};

    #[derive(Debug, serde::Deserialize)]
    struct Holder {
        ttl: Duration,
    }

    fn extract(toml: &str) -> Result<Duration, String> {
        Figment::from(Toml::string(toml))
            .extract::<Holder>()
            .map(|h| h.ttl)
            .map_err(|e| e.to_string())
    }

    #[test]
    fn parses_days_and_compound_values() {
        assert_eq!(
            extract("ttl = \"7d\"").unwrap().as_std().as_secs(),
            7 * 86_400
        );
        assert_eq!(
            extract("ttl = \"1h30m\"").unwrap().as_std().as_secs(),
            5_400
        );
    }

    #[test]
    fn rejects_bare_integer() {
        let err = extract("ttl = 15").unwrap_err().to_string();
        assert!(err.contains("duration"), "{err}");
    }

    #[test]
    fn rejects_unitless_string() {
        assert!("15".parse::<Duration>().is_err());
    }

    #[test]
    fn display_round_trips() {
        let d: Duration = "1h30m".parse().unwrap();
        assert_eq!(d.to_string().parse::<Duration>().unwrap(), d);
    }

    #[test]
    fn display_is_compact_and_exact() {
        for (input, shown) in [
            ("30d", "30d"),
            ("180d", "180d"),
            ("60s", "1m"),
            ("1h30m", "1h30m"),
            ("1d 1s", "1d1s"),
            ("1500ms", "1s500ms"),
            ("2us", "2us"),
            ("0", "0s"),
        ] {
            let d: Duration = input.parse().unwrap();
            assert_eq!(d.to_string(), shown, "{input}");
            assert_eq!(shown.parse::<Duration>().unwrap(), d, "{input}");
        }
    }
}
