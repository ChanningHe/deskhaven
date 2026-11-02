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

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", humantime::format_duration(self.0))
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
}
