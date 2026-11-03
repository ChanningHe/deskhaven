//! Windows file names the client reads its server from (`src/custom_server.rs:44-113`,
//! used at startup and by `--config`, `src/core_main.rs:488-512`).
//!
//! - Plain: a name containing `host=` is cut at commas from there; tokens starting with
//!   `host=`, `key=`, `api=` and `relay=` (case-insensitive) set the fields
//!   (`custom_server.rs:64-92`). Windows renames duplicates to `name (1).exe`, so the
//!   template ends in a comma, the final delimiter (`custom_server.rs:52-63`).
//! - Encoded: `rustdesk-licensed-<config string>.exe`; `-licensed-` is rewritten to `--`
//!   and the name split there (`custom_server.rs:94-110`, a `(` suffix is tolerated).
//!
//! A Windows file name cannot hold `\ / : * ? " < > |`, so the plain form cannot carry an
//! API URL, a custom port, an IPv6 address or a key with a `/` (standard base64 has one in
//! about half of all keys). The encoded form holds everything.

use super::ServerConfig;

/// Longest file name on NTFS, in UTF-16 units (all ASCII here).
const MAX_FILE_NAME: usize = 255;
const FORBIDDEN: &str = "\\/:*?\"<>|,";

/// The two file name templates; `None` where the values do not fit the form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsNames {
    /// `rustdesk-host=…,key=…,api=…,relay=…,.exe`, readable and editable by hand.
    pub plain: Option<String>,
    /// `rustdesk-licensed-<config string>.exe`.
    pub encoded: Option<String>,
}

impl ServerConfig {
    pub fn windows_names(&self) -> WindowsNames {
        WindowsNames {
            plain: self.plain_name(),
            encoded: self.encoded_name(),
        }
    }

    fn plain_name(&self) -> Option<String> {
        if self.host.is_empty() || !self.values().iter().all(|v| fits_file_name(v)) {
            return None;
        }
        let tokens: String = [
            ("host", &self.host),
            ("key", &self.key),
            ("api", &self.api),
            ("relay", &self.relay),
        ]
        .into_iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(name, value)| format!("{name}={value},"))
        .collect();
        short_enough(format!("rustdesk-{tokens}.exe"))
    }

    fn encoded_name(&self) -> Option<String> {
        let string = self.config_string().ok()?;
        short_enough(format!("rustdesk-licensed-{string}.exe"))
    }
}

/// Printable ASCII without the characters Windows and the comma tokenizer forbid.
fn fits_file_name(value: &str) -> bool {
    value
        .chars()
        .all(|c| c.is_ascii_graphic() && !FORBIDDEN.contains(c))
}

fn short_enough(name: String) -> Option<String> {
    (name.len() <= MAX_FILE_NAME).then_some(name)
}
