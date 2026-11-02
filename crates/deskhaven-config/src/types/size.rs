use std::fmt;
use std::str::FromStr;

/// A byte size written with units, e.g. `"10MiB"` or `"512KB"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Size(u64);

impl Size {
    pub const fn from_bytes(bytes: u64) -> Self {
        Self(bytes)
    }

    pub const fn bytes(&self) -> u64 {
        self.0
    }
}

/// Parse failure for [`Size`].
#[derive(Debug, thiserror::Error)]
#[error("invalid size {input:?}: {reason} (expected e.g. \"10MiB\")")]
pub struct SizeError {
    input: String,
    reason: String,
}

impl FromStr for Size {
    type Err = SizeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = |reason: String| SizeError {
            input: s.to_owned(),
            reason,
        };
        let trimmed = s.trim();
        if trimmed.chars().all(|c| c.is_ascii_digit()) {
            return Err(err("a unit is required".into()));
        }
        trimmed
            .parse::<bytesize::ByteSize>()
            .map(|b| Self(b.as_u64()))
            .map_err(err)
    }
}

/// Largest binary unit first; used for exact (lossless) display.
const IEC_UNITS: [(&str, u64); 4] = [
    ("TiB", 1 << 40),
    ("GiB", 1 << 30),
    ("MiB", 1 << 20),
    ("KiB", 1 << 10),
];

impl fmt::Display for Size {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let exact = IEC_UNITS
            .iter()
            .find(|(_, n)| self.0 >= *n && self.0.is_multiple_of(*n));
        match exact {
            Some((unit, n)) => write!(f, "{}{unit}", self.0 / n),
            None => write!(f, "{}B", self.0),
        }
    }
}

super::string_serde!(Size, "a size string with units, e.g. \"10MiB\"");
super::string_schema!(Size, "Size", "Byte size with units", "10MiB");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_binary_and_decimal_units() {
        assert_eq!("10MiB".parse::<Size>().unwrap().bytes(), 10 * 1024 * 1024);
        assert_eq!("1KB".parse::<Size>().unwrap().bytes(), 1000);
    }

    #[test]
    fn rejects_missing_unit_and_garbage() {
        assert!("1024".parse::<Size>().is_err());
        assert!("ten".parse::<Size>().is_err());
    }

    #[test]
    fn display_round_trips() {
        for input in ["10MiB", "1500B", "3GiB", "0B"] {
            let s: Size = input.parse().unwrap();
            assert_eq!(s.to_string(), input);
            assert_eq!(s.to_string().parse::<Size>().unwrap(), s);
        }
    }
}
