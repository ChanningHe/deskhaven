//! The settings registry: declarations, defaults, typed access, schema export.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeSet;

use deskhaven_config::settings::{
    self, AcceptPresets, AclAction, HostPort, Registration, SmtpTls, Snapshot, TfaEnforce,
};
use deskhaven_config::{Duration, Source};
use serde_json::{Value as Json, json};

const GOLDEN_SCHEMA: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/snapshots/settings_schema.json"
);
const UPDATE_ENV: &str = "DESKHAVEN_UPDATE_SNAPSHOTS";

fn d(s: &str) -> Duration {
    s.parse().unwrap()
}

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
fn only_smtp_password_is_sensitive() {
    let sensitive: Vec<&str> = settings::all()
        .iter()
        .filter(|s| s.is_sensitive())
        .map(|s| s.key())
        .collect();
    assert_eq!(sensitive, ["smtp.password"]);
}

#[test]
fn typed_access_returns_defaults() {
    let s = Snapshot::defaults();
    assert_eq!(s.get(&settings::AUTH_SESSION_TTL), d("30d"));
    assert_eq!(s.get(&settings::AUTH_SESSION_IDLE_TTL), d("7d"));
    assert_eq!(s.get(&settings::AUTH_PASSWORD_MIN_LENGTH), 10);
    assert_eq!(s.get(&settings::AUTH_REGISTRATION), Registration::Disabled);
    assert_eq!(s.get(&settings::AUTH_TFA_ENFORCE), TfaEnforce::Off);
    assert!(!s.get(&settings::AUTH_EMAIL_CODE_ENABLED));
    assert_eq!(s.get(&settings::SMTP_TLS), SmtpTls::Starttls);
    assert_eq!(s.get(&settings::SMTP_HOST), "");
    assert!(s.get(&settings::SMTP_PASSWORD).is_empty());
    assert_eq!(
        s.get(&settings::SIGNAL_RELAY_SERVERS),
        Vec::<HostPort>::new()
    );
    assert_eq!(s.get(&settings::SIGNAL_ACL_DEFAULT), AclAction::Allow);
    assert_eq!(s.get(&settings::SIGNAL_ACL_ON_ERROR), AclAction::Deny);
    assert_eq!(s.get(&settings::DEVICES_ACCEPT_PRESETS), AcceptPresets::Off);
    assert_eq!(s.get(&settings::AUDIT_RETENTION_LOGIN), d("180d"));
}

#[test]
fn schema_lists_type_constraints_and_rules() {
    let schema = settings::schema();
    let ttl = &schema["settings"][0];
    assert_eq!(ttl["key"], json!("auth.session_ttl"));
    assert_eq!(ttl["type"], json!("duration"));
    assert_eq!(ttl["min"], json!("5m"));
    assert_eq!(ttl["category"], json!("auth"));
    let registration = schema["settings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["key"] == json!("auth.registration"))
        .unwrap();
    assert_eq!(
        registration["variants"],
        json!(["disabled", "invite", "open"])
    );
    assert!(!schema["rules"].as_array().unwrap().is_empty());
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
