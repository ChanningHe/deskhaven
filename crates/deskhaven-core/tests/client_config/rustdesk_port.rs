//! A port of the official RustDesk client's server-configuration decoders at the pinned
//! commit 23e5bb4, to check our encoder against what the client really accepts. Not
//! built from the client's code; the logic is transcribed and only the unsigned branch is
//! kept (the signed branch needs libsodium and serves RustDesk Pro's own licence strings).
//!
//! - Import box, deep link and mobile QR scan: `flutter/lib/common.dart:2938-2952`
//!   (`ServerConfig.decode`, called by `importConfig` at `common.dart:3551-3556`); the QR
//!   payload prefix `config=` is `flutter/lib/mobile/pages/scan_page.dart:152-157`.
//! - `rustdesk --config <string>` and the renamed exe: `src/core_main.rs:488-512` calls
//!   `src/custom_server.rs:44-113` (`get_custom_server_from_string`) with the string plus
//!   `.exe`; the encoded part is decoded at `src/custom_server.rs:24-42`.

#![allow(clippy::unwrap_used, clippy::panic)]

use base64_022::Engine as _;
use base64_022::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};

/// `CustomServer` (`custom_server.rs:12-22`).
#[derive(Debug, PartialEq, Eq, Default, Clone)]
pub struct Decoded {
    pub key: String,
    pub host: String,
    pub api: String,
    pub relay: String,
}

fn field(json: &serde_json::Value, name: &str) -> String {
    json.get(name)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_owned()
}

fn from_json(data: &[u8]) -> Option<Decoded> {
    let json: serde_json::Value = serde_json::from_slice(data).ok()?;
    json.is_object().then(|| Decoded {
        key: field(&json, "key"),
        host: field(&json, "host"),
        api: field(&json, "api"),
        relay: field(&json, "relay"),
    })
}

/// `get_custom_server_from_config_string` (`custom_server.rs:24-42`), unsigned branch.
fn from_config_string(s: &str) -> Option<Decoded> {
    let reversed: String = s.chars().rev().collect();
    let data = URL_SAFE_NO_PAD
        .decode(&reversed)
        .or_else(|_| URL_SAFE.decode(&reversed))
        .ok()?;
    from_json(&data)
}

/// `get_custom_server_from_string` (`custom_server.rs:44-113`); `s` is a file name or the
/// `--config` argument with `.exe` appended.
pub fn from_file_name(s: &str) -> Option<Decoded> {
    let s = if s.to_lowercase().ends_with(".exe.exe") {
        &s[0..s.len() - 8]
    } else if s.to_lowercase().ends_with(".exe") {
        &s[0..s.len() - 4]
    } else {
        s
    };
    if s.to_lowercase().contains("host=") {
        return Some(from_host_tokens(s));
    }
    let s = s
        .replace("-licensed---", "--")
        .replace("-licensed--", "--")
        .replace("-licensed-", "--");
    for piece in s.split("--") {
        if let Some(lic) = from_config_string(piece.trim()) {
            return Some(lic);
        }
        if piece.contains('(') {
            for piece in piece.split('(') {
                if let Some(lic) = from_config_string(piece.trim()) {
                    return Some(lic);
                }
            }
        }
    }
    None
}

/// The `host=` branch (`custom_server.rs:64-92`).
fn from_host_tokens(s: &str) -> Decoded {
    let stripped = &s[s.to_ascii_lowercase().find("host=").unwrap_or(0)..s.len()];
    let mut out = Decoded::default();
    for el in stripped.split(',') {
        let lower = el.to_lowercase();
        if lower.starts_with("host=") {
            out.host = el.chars().skip(5).collect();
        }
        if lower.starts_with("key=") {
            out.key = el.chars().skip(4).collect();
        }
        if lower.starts_with("api=") {
            out.api = el.chars().skip(4).collect();
        }
        if lower.starts_with("relay=") {
            out.relay = el.chars().skip(6).collect();
        }
    }
    out
}

/// `rustdesk --config <arg>` (`core_main.rs:488-512`): the argument becomes an exe name.
pub fn cli_config(arg: &str) -> Option<Decoded> {
    let name = if arg.ends_with(".exe") {
        arg.to_owned()
    } else {
        format!("{arg}.exe")
    };
    from_file_name(&name).filter(|lic| !lic.host.is_empty())
}

/// `ServerConfig.decode` (`common.dart:2938-2952`) after `importConfig` trimmed the text
/// (`common.dart:3551-3556`): plain JSON first, else the reversed text as base64 with the
/// padding normalized (Dart accepts both alphabets).
pub fn flutter_decode(text: &str) -> Option<Decoded> {
    let text = text.trim();
    if let Some(decoded) = from_json(text.as_bytes()) {
        return Some(decoded);
    }
    let reversed: String = text.chars().rev().collect();
    let data = URL_SAFE_NO_PAD
        .decode(reversed.trim_end_matches('='))
        .ok()?;
    from_json(&data)
}

/// The mobile QR scan (`scan_page.dart:152-157`): `config=` followed by the string.
pub fn qr_decode(payload: &str) -> Option<Decoded> {
    flutter_decode(payload.strip_prefix("config=")?)
}
