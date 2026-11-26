//! The settings registry: declarations, defaults, typed access, schema export.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeSet;

use deskhaven_config::Source;
use deskhaven_config::settings::{self, Snapshot};
use serde_json::{Value as Json, json};

const GOLDEN_SCHEMA: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/snapshots/settings_schema.json"
);
const UPDATE_ENV: &str = "DESKHAVEN_UPDATE_SNAPSHOTS";

/// 05-config §3.6, one line per key: (key, public JSON form of the default).
const P0_DEFAULTS: &[(&str, &str)] = &[
    ("auth.session_ttl", r#""30d""#),
    ("auth.session_idle_ttl", r#""7d""#),
    ("auth.max_sessions_per_user", "1000"),
    ("auth.password.min_length", "10"),
    ("auth.lockout.max_failures", "5"),
    ("auth.lockout.window", r#""15m""#),
    ("auth.lockout.duration", r#""15m""#),
    ("auth.ip_rate_limit.per_minute", "20"),
    ("auth.registration", r#""disabled""#),
    ("auth.tfa.enforce", r#""off""#),
    ("auth.email_code.enabled", "false"),
    ("auth.oidc.link_email_max_role", r#""user""#),
    ("smtp.host", r#""""#),
    ("smtp.port", "587"),
    ("smtp.tls", r#""starttls""#),
    ("smtp.username", r#""""#),
    ("smtp.password", r#"{"set":false}"#),
    ("smtp.from", r#""""#),
    ("devices.verify_uuid", "true"),
    ("devices.require_deploy", "false"),
    ("devices.last_seen_flush", r#""1m""#),
    ("devices.accept_presets", r#""off""#),
    ("addressbook.max_peers", "1000"),
    ("audit.retention.connection", r#""365d""#),
    ("audit.retention.file", r#""365d""#),
    ("audit.retention.alarm", r#""365d""#),
    ("audit.retention.login", r#""180d""#),
    ("audit.retention.console", r#""730d""#),
    ("signal.relay_servers", "[]"),
    ("signal.always_use_relay", "false"),
    ("signal.require_kx_v1", "false"),
    ("signal.acl.default", r#""allow""#),
    ("signal.acl.on_error", r#""deny""#),
    ("updates.check", "true"),
    ("web_client.enabled", "false"),
    (
        "web_client.ice_servers",
        r#""stun:stun.cloudflare.com:3478,stun:stun.l.google.com:19302,stun:stun.antisip.com:3478,stun:stun.nextcloud.com:443""#,
    ),
    ("web_client.turn_username", r#""""#),
    ("web_client.turn_credential", r#"{"set":false}"#),
];

#[test]
fn every_p0_setting_is_declared_with_its_default() {
    let declared: Vec<&str> = settings::all().iter().map(|s| s.key()).collect();
    let expected: Vec<&str> = P0_DEFAULTS.iter().map(|(k, _)| *k).collect();
    assert_eq!(declared, expected);

    let snapshot = Snapshot::defaults();
    for (key, default) in P0_DEFAULTS {
        let view = snapshot.view(key).unwrap();
        let expected: Json = serde_json::from_str(default).unwrap();
        assert_eq!(view.value, expected, "{key}");
        assert_eq!(view.default, expected, "{key}");
        assert_eq!(view.source, Source::Default, "{key}");
        assert!(!view.pinned, "{key}");
        assert_eq!(view.version, 0, "{key}");
    }
}

#[test]
fn keys_are_unique_and_well_formed() {
    let mut seen = BTreeSet::new();
    for setting in settings::all() {
        let key = setting.key();
        assert!(seen.insert(key), "duplicate key {key}");
        let segments: Vec<&str> = key.split('.').collect();
        assert!(segments.len() >= 2, "{key}: needs a category prefix");
        assert!(
            segments.iter().all(|s| !s.is_empty()
                && !s.contains("__")
                && !s.ends_with('_')
                && s.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')),
            "{key}: segments must be lower_snake_case"
        );
        assert_eq!(setting.category(), segments[0], "{key}");
        assert!(!setting.description().is_empty(), "{key}: description");
    }
}

#[test]
fn every_default_satisfies_its_own_rule() {
    for setting in settings::all() {
        let default = setting.default_value();
        let raw = default.to_json();
        if setting.is_sensitive() {
            continue;
        }
        assert_eq!(
            setting.coerce(&raw).as_ref(),
            Ok(&default),
            "{}",
            setting.key()
        );
    }
}

#[test]
fn only_passwords_and_credentials_are_sensitive() {
    let sensitive: Vec<&str> = settings::all()
        .iter()
        .filter(|s| s.is_sensitive())
        .map(|s| s.key())
        .collect();
    assert_eq!(sensitive, ["smtp.password", "web_client.turn_credential"]);
}

/// A row that cannot be read turns these features off rather than on.
#[test]
fn unreadable_feature_rows_turn_the_feature_off() {
    for key in ["web_client.enabled", "updates.check"] {
        let setting = settings::lookup(key).unwrap();
        assert_eq!(setting.bad_row_value().to_json(), json!(false), "{key}");
    }
}

/// ICE server URLs: `stun:` / `stuns:` / `turn:` / `turns:` with a host, an optional port and
/// (TURN only) an optional transport, comma separated.
#[test]
fn ice_servers_accept_stun_and_turn_urls_only() {
    let setting = settings::lookup("web_client.ice_servers").unwrap();
    for good in [
        "",
        "stun:stun.example.com",
        "stun:192.0.2.1:3478, turn:[2001:db8::1]:3478?transport=tcp",
        "turns:turn.example.com:5349?transport=tcp,stuns:stun.example.com:5349",
    ] {
        assert!(setting.coerce(&json!(good)).is_ok(), "{good:?}");
    }
    for bad in [
        "http://stun.example.com",
        "stun:",
        "stun://stun.example.com",
        "stun:stun.example.com:0",
        "stun:stun.example.com?transport=udp",
        "turn:turn.example.com?transport=sctp",
        "stun:a.example,,stun:b.example",
        "stun:bad host",
        "turn:user@turn.example.com",
    ] {
        assert!(setting.coerce(&json!(bad)).is_err(), "{bad:?}");
    }
    let nine = ["stun:s.example"; 9].join(",");
    assert!(setting.coerce(&json!(nine)).is_err(), "at most 8 servers");
}

/// Golden file; regenerate with `DESKHAVEN_UPDATE_SNAPSHOTS=1 cargo test`.
#[test]
fn schema_export_matches_snapshot() {
    let actual = settings::schema();
    let rendered = serde_json::to_string_pretty(&actual).unwrap() + "\n";
    if std::env::var(UPDATE_ENV).as_deref() == Ok("1") {
        std::fs::write(GOLDEN_SCHEMA, &rendered).unwrap();
        return;
    }
    let golden = std::fs::read_to_string(GOLDEN_SCHEMA)
        .unwrap_or_else(|e| panic!("{GOLDEN_SCHEMA}: {e}; run with {UPDATE_ENV}=1"));
    let expected: Json = serde_json::from_str(&golden).unwrap();
    assert_eq!(
        actual, expected,
        "schema changed; rerun with {UPDATE_ENV}=1"
    );
}

/// The TURN credential goes to one TURN server only: with it set, every `turn:` / `turns:`
/// URL must name the same host.
#[test]
fn a_turn_credential_needs_a_single_turn_host() {
    let check = |ice: &str, credential: &str| {
        let changes = [
            ("web_client.ice_servers", json!(ice)),
            ("web_client.turn_credential", json!(credential)),
        ]
        .map(|(key, value)| settings::Change {
            key: key.to_owned(),
            value,
            version: 0,
        });
        let accepted = settings::prepare(&settings::PinSet::empty(), &changes).unwrap();
        let next = Snapshot::defaults().with_accepted(&accepted, None, 0);
        settings::check_rules(&next, &["web_client.turn_credential"])
    };
    let one_host = "stun:s.example,turn:t.example:3478,turns:T.example:5349?transport=tcp";
    let two_hosts = "turn:t.example:3478,turn:other.example:3478";

    assert!(check(one_host, "s3cret").is_ok());
    assert!(
        check(two_hosts, "").is_ok(),
        "no credential, nothing to leak"
    );
    let rejected = check(two_hosts, "s3cret").unwrap_err();
    assert!(format!("{rejected:?}").contains("one host"), "{rejected:?}");
}
