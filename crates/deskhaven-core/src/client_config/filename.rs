//! Windows renames duplicates to `name (1).exe`, so the plain template ends in a comma, the
//! final delimiter. A file name cannot hold `\ / : * ? " < > |`, so the plain form cannot carry
//! an API URL, a custom port, an IPv6 address or a key with a `/`; the encoded form holds
//! everything.

use super::ServerConfig;

/// Longest file name on NTFS, in UTF-16 units (all ASCII here).
const MAX_FILE_NAME: usize = 255;
const FORBIDDEN: &str = "\\/:*?\"<>|,";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsNames {
    pub plain: Option<String>,
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

fn fits_file_name(value: &str) -> bool {
    value
        .chars()
        .all(|c| c.is_ascii_graphic() && !FORBIDDEN.contains(c))
}

fn short_enough(name: String) -> Option<String> {
    (name.len() <= MAX_FILE_NAME).then_some(name)
}
