//! The RustDesk client configuration (M3-11): the config string, the QR payload and the
//! Windows file names must be accepted by the official client. The client's decoders are
//! ported in `rustdesk_port` (pinned commit 23e5bb4); the golden strings below were made
//! independently of this code (Python mimicking Dart's `ServerConfig.encode`:
//! `common.dart:2956-2966`).
#![allow(clippy::unwrap_used, clippy::panic)]

mod resolve;
mod rustdesk_port;

use base64_022::Engine as _;
use base64_022::engine::general_purpose::URL_SAFE;
use deskhaven_core::client_config::{ConfigError, ServerConfig};
use rustdesk_port::{Decoded, cli_config, flutter_decode, from_file_name, qr_decode};

pub const KEY: &str = "5Qbwsde3unUcJBtrx9ZkvUmwFNoExHzpryHuPUdqlWM=";

const GOLDEN_HTTPS: &str = "9JSPNdFbxRWVQVHS5JHc6hEeF9mTGdXbVZ3aalDeyRnQKNWVuV3MlR2c3JWU1IiOikXZrJCLi02bj5SZsBXbhhXZuQmcv8iOzBHd0hmI6ISawFmIsIiI6ISehxWZyJCLi02bj5SZsBXbhhXZuQmciojI0N3boJye";
const GOLDEN_PORTS: &str = "=0nI900VsFHZVBVdIlncwpHS4V0bOZ0dtVldrpVO4JHdCp0YV5WdzUGZzdnYRVjI6ISeltmIsICNxETMyozNuIjLw4iM5EzLvoDc0RHaiojIpBXYiwiI3ITMxIjOt92YuUGbw1WY4VmL5FGblJnI6ISehxWZyJCLiYjMxEjM602bj5SZsBXbhhXZuQmciojI0N3boJye";
const GOLDEN_HOST_ONLY: &str =
    "9JiI6ISeltmIsIiI6ISawFmIsIiI6ISehxWZyJCLi02bj5SZsBXbhhXZuQmciojI0N3boJye";

pub fn server(host: &str, relay: &str, api: &str, key: &str) -> ServerConfig {
    ServerConfig {
        host: host.to_owned(),
        relay: relay.to_owned(),
        api: api.to_owned(),
        key: key.to_owned(),
    }
}

fn decoded(s: &ServerConfig) -> Decoded {
    Decoded {
        key: s.key.clone(),
        host: s.host.clone(),
        api: s.api.clone(),
        relay: s.relay.clone(),
    }
}

/// Dart's `ServerConfig.encode` (`common.dart:2956-2966`): compact JSON in the order
/// host, relay, api, key; padded base64url; reversed.
fn canonical(s: &ServerConfig) -> String {
    let json = format!(
        r#"{{"host":"{}","relay":"{}","api":"{}","key":"{}"}}"#,
        s.host, s.relay, s.api, s.key
    );
    URL_SAFE.encode(json).chars().rev().collect()
}

#[test]
fn matches_the_golden_strings() {
    let https = server("rd.example.com", "", "https://rd.example.com", KEY);
    let ports = server(
        "rd.example.com:21126",
        "relay.example.com:21127",
        "http://192.0.2.7:21114",
        KEY,
    );
    let host_only = server("rd.example.com", "", "", "");

    assert_eq!(https.config_string().unwrap(), GOLDEN_HTTPS);
    assert_eq!(ports.config_string().unwrap(), GOLDEN_PORTS);
    assert_eq!(host_only.config_string().unwrap(), GOLDEN_HOST_ONLY);
}

#[test]
fn the_clients_own_test_vector_decodes_to_what_we_encode() {
    // `custom_server.rs:190-198` of the client: the licensed string of this config.
    let official = "=0nI900VsFHZVBVdIlncwpHS4V0bOZ0dtVldrpVO4JHdCp0YV5WdzUGZzdnYRVjI6ISeltmIsISMuEjLx4SMiojI0N3boJye";
    let ours = server("1.1.1.1", "", "", KEY);

    assert_eq!(cli_config(official).unwrap(), decoded(&ours));
    assert_eq!(
        cli_config(&ours.config_string().unwrap()).unwrap(),
        decoded(&ours)
    );
}

#[test]
fn every_client_path_decodes_the_string() {
    let cfg = server(
        "rd.example.com:21126",
        "relay.example.com",
        "https://rd.example.com/deskhaven",
        KEY,
    );

    let string = cfg.config_string().unwrap();
    let qr = cfg.qr_payload().unwrap();

    assert_eq!(flutter_decode(&string).unwrap(), decoded(&cfg));
    assert_eq!(cli_config(&string).unwrap(), decoded(&cfg));
    assert_eq!(qr, format!("config={string}"));
    assert_eq!(qr_decode(&qr).unwrap(), decoded(&cfg));
}

#[test]
fn the_string_never_holds_the_separator_the_cli_splits_at() {
    // `--config` and the file name parser cut at "--" (`custom_server.rs:98`). Even the
    // characters that encode to '-' (0x3E, 0x7E) cannot make two in a row.
    for n in 0..3000 {
        let tail = "~>".repeat(n % 7);
        let cfg = server(
            &format!("h{n}.example.com"),
            "",
            &format!("https://x.test/{tail}{n}"),
            KEY,
        );

        let string = cfg.config_string().unwrap();

        assert_eq!(string, canonical(&cfg));
        assert!(
            !string.contains("--") && !string.starts_with('-'),
            "{string}"
        );
    }
}

#[test]
fn many_configs_round_trip_through_every_path() {
    for n in 0..600 {
        let cfg = server(
            &format!("rd{n}.example.com"),
            if n % 3 == 0 { "relay.example.com" } else { "" },
            &format!("https://rd{n}.example.com"),
            KEY,
        );
        let string = cfg.config_string().unwrap();
        assert_eq!(cli_config(&string).unwrap(), decoded(&cfg), "{n}");
        assert_eq!(flutter_decode(&string).unwrap(), decoded(&cfg), "{n}");
        let names = cfg.windows_names();
        let encoded = names.encoded.unwrap();
        assert_eq!(
            from_file_name(&encoded).unwrap(),
            decoded(&cfg),
            "{encoded}"
        );
    }
}

#[test]
fn a_config_without_a_host_has_no_string() {
    let cfg = server("", "", "https://rd.example.com", KEY);

    assert_eq!(cfg.config_string(), Err(ConfigError::MissingHost));
    assert_eq!(cfg.qr_payload(), Err(ConfigError::MissingHost));
    assert_eq!(cfg.windows_names().encoded, None);
}

#[test]
fn non_ascii_values_are_refused() {
    // Dart encodes `jsonEncode(..).codeUnits`, which breaks above U+00FF.
    let cfg = server("rd.exämple.com", "", "", KEY);

    assert_eq!(cfg.config_string(), Err(ConfigError::NotAscii));
}

#[test]
fn the_plain_file_name_carries_what_windows_allows() {
    let cfg = server("rd.example.com", "", "", "abcDEF123+xyz=");

    let names = cfg.windows_names();
    let plain = names.plain.unwrap();

    assert_eq!(
        plain,
        "rustdesk-host=rd.example.com,key=abcDEF123+xyz=,.exe"
    );
    assert_eq!(from_file_name(&plain).unwrap(), decoded(&cfg));
}

#[test]
fn the_plain_file_name_is_absent_when_a_value_cannot_be_in_a_windows_name() {
    // `:` and `/` of an API URL, `/` inside a base64 key: `\ / : * ? " < > |`.
    let with_api = server("rd.example.com", "", "https://rd.example.com", "abc");
    let slash_key = server("rd.example.com", "", "", "ab/cd+ef=");
    let comma = server("rd.example.com", "", "", "a,b");
    let port = server("rd.example.com:21126", "", "", "abc");
    let ipv6 = server("[2001:db8::1]", "", "", "abc");

    assert_eq!(port.windows_names().plain, None);
    assert_eq!(ipv6.windows_names().plain, None);
    assert!(port.windows_names().encoded.is_some());
    assert_eq!(with_api.windows_names().plain, None);
    assert_eq!(slash_key.windows_names().plain, None);
    assert_eq!(comma.windows_names().plain, None);
    assert!(with_api.windows_names().encoded.is_some());
    assert!(slash_key.windows_names().encoded.is_some());
}

#[test]
fn the_plain_name_orders_host_key_api_relay_and_skips_empty_values() {
    let cfg = server("rd.example.com", "relay.example.com", "", KEY);

    let plain = cfg.windows_names().plain;

    // `custom_server.rs` finds `host=` first and reads the other keys from the tokens
    // after it; the template puts host first and a trailing comma last (`(1)` renames).
    assert_eq!(
        plain.as_deref(),
        Some(
            format!("rustdesk-host=rd.example.com,key={KEY},relay=relay.example.com,.exe").as_str()
        )
    );
    // The key may contain '/': then this very name is impossible, checked above.
}

#[test]
fn encoded_names_survive_windows_duplicate_suffixes() {
    let cfg = server("rd.example.com", "", "https://rd.example.com", KEY);
    let encoded = cfg.windows_names().encoded.unwrap();
    let stem = encoded.strip_suffix(".exe").unwrap();

    // Windows appends " (1)" before the extension (`custom_server.rs:102-107`).
    let renamed = format!("{stem} (1).exe");

    assert!(encoded.starts_with("rustdesk-licensed-") && encoded.ends_with(".exe"));
    assert_eq!(from_file_name(&renamed).unwrap(), decoded(&cfg));
}
