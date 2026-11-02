//! Device-reported system information (`POST /api/sysinfo`, 03 §2.2;
//! `src/hbbs_http/sync.rs:135-228`, `src/common.rs:962-1006` `get_sysinfo`) and its content
//! hash.
//!
//! Every field is self-reported by an unauthenticated device (06 §2): control characters are
//! dropped (PostgreSQL TEXT cannot even hold NUL) and each field is cut to
//! [`MAX_FIELD_CHARS`] characters. A truncated value is still stored: refusing would leave the
//! client retrying every 120 s forever without its "Pro" features.

use data_encoding::HEXLOWER;
use sha2::{Digest, Sha256};

/// Longest stored sysinfo field, in characters (a hostname is at most 253 bytes; real
/// `os` / `cpu` strings are well under 100).
pub const MAX_FIELD_CHARS: usize = 256;
/// Domain separation and format version of [`Sysinfo::hash`].
const HASH_DOMAIN: &[u8] = b"deskhaven/sysinfo/v1";

/// The stored sysinfo columns of `devices` (04 §4). `None`: not reported.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sysinfo {
    pub hostname: Option<String>,
    pub username: Option<String>,
    pub os: Option<String>,
    pub cpu: Option<String>,
    pub memory: Option<String>,
    /// Client version (`crate::VERSION`).
    pub version: Option<String>,
}

impl Sysinfo {
    /// Every field cleaned as described in the module documentation.
    pub fn sanitized(self) -> Self {
        Self {
            hostname: self.hostname.map(clean),
            username: self.username.map(clean),
            os: self.os.map(clean),
            cpu: self.cpu.map(clean),
            memory: self.memory.map(clean),
            version: self.version.map(clean),
        }
    }

    /// Content hash stored in `devices.sysinfo_hash`: lower-case hex SHA-256 over every field,
    /// each encoded unambiguously (absent and empty differ, no field can bleed into the next).
    pub fn hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(HASH_DOMAIN);
        for field in self.fields() {
            match field {
                None => hasher.update([0]),
                Some(value) => {
                    hasher.update([1]);
                    hasher.update((value.len() as u64).to_le_bytes());
                    hasher.update(value.as_bytes());
                }
            }
        }
        HEXLOWER.encode(&hasher.finalize())
    }

    fn fields(&self) -> [Option<&str>; 6] {
        [
            self.hostname.as_deref(),
            self.username.as_deref(),
            self.os.as_deref(),
            self.cpu.as_deref(),
            self.memory.as_deref(),
            self.version.as_deref(),
        ]
    }
}

/// Without control characters, at most [`MAX_FIELD_CHARS`] characters.
fn clean(raw: String) -> String {
    let fits = raw.chars().count() <= MAX_FIELD_CHARS;
    if fits && !raw.chars().any(char::is_control) {
        return raw;
    }
    raw.chars()
        .filter(|c| !c.is_control())
        .take(MAX_FIELD_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(hostname: &str) -> Sysinfo {
        Sysinfo {
            hostname: Some(hostname.to_owned()),
            os: Some("linux / Ubuntu 24.04".to_owned()),
            ..Default::default()
        }
    }

    #[test]
    fn hash_is_stable_hex_sha256() {
        let hash = info("pc").hash();

        assert_eq!(hash, info("pc").hash());
        assert_eq!(hash.len(), 64);
        assert!(
            hash.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
    }

    #[test]
    fn hash_changes_with_any_field() {
        let base = info("pc");
        let mut changed = base.clone();
        changed.version = Some("1.4.1".to_owned());

        assert_ne!(base.hash(), info("pc2").hash());
        assert_ne!(base.hash(), changed.hash());
    }

    #[test]
    fn hash_tells_absent_from_empty_and_fields_apart() {
        let absent = Sysinfo::default();
        let empty = Sysinfo {
            hostname: Some(String::new()),
            ..Default::default()
        };
        let in_hostname = Sysinfo {
            hostname: Some("ab".to_owned()),
            ..Default::default()
        };
        let split = Sysinfo {
            hostname: Some("a".to_owned()),
            username: Some("b".to_owned()),
            ..Default::default()
        };

        assert_ne!(absent.hash(), empty.hash());
        assert_ne!(in_hostname.hash(), split.hash());
    }

    #[test]
    fn sanitized_drops_control_characters_and_truncates() {
        let long = "é".repeat(MAX_FIELD_CHARS + 10);
        let raw = Sysinfo {
            hostname: Some("pc\u{0}-1\n".to_owned()),
            os: Some(long),
            cpu: Some("Intel, 3.2GHz, 8/4 cores".to_owned()),
            ..Default::default()
        };

        let clean = raw.sanitized();

        assert_eq!(clean.hostname.as_deref(), Some("pc-1"));
        assert_eq!(clean.os.unwrap().chars().count(), MAX_FIELD_CHARS);
        assert_eq!(clean.cpu.as_deref(), Some("Intel, 3.2GHz, 8/4 cores"));
        assert_eq!(clean.username, None);
    }
}
