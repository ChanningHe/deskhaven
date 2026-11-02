//! Pure update validation and snapshot building (no database).

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::panic)]

use deskhaven_config::Source;
use deskhaven_config::settings::{
    self, Change, PinSet, Rejection, Snapshot, Stored, Update, check_rules, prepare,
};
use serde_json::{Value as Json, json};

fn change(key: &str, value: Json) -> Change {
    Change {
        key: key.to_owned(),
        value,
        version: 0,
    }
}

fn stored(key: &str, value: Json, version: i64) -> Stored {
    Stored {
        key: key.to_owned(),
        value,
        version,
        by: Some(7),
        at: 1_700_000_000_000,
    }
}

fn invalid_keys(rejection: Rejection) -> Vec<String> {
    match rejection {
        Rejection::Invalid { problems } => problems.into_iter().map(|p| p.key).collect(),
        other => panic!("expected Invalid, got {other:?}"),
    }
}

/// Applies accepted changes as database updates and checks cross-key rules.
fn candidate(base: &Snapshot, changes: &[Change]) -> Result<Snapshot, Rejection> {
    let accepted = prepare(&PinSet::empty(), changes)?;
    let updates = accepted.into_iter().map(|a| Update {
        key: a.key(),
        value: a.into_value(),
        source: Source::Database {
            version: 1,
            by: None,
            at: 0,
        },
    });
    let next = base.with_updates(updates);
    let keys: Vec<&str> = changes.iter().map(|c| c.key.as_str()).collect();
    check_rules(&next, &keys)?;
    Ok(next)
}

#[test]
fn database_values_override_defaults_with_source() {
    let rows = vec![
        stored("auth.session_ttl", json!("60d"), 3),
        stored("auth.registration", json!("open"), 1),
    ];

    let (s, warnings) = Snapshot::build(&PinSet::empty(), rows);

    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        s.get(&settings::AUTH_SESSION_TTL).as_std().as_secs(),
        60 * 86_400
    );
    let view = s.view("auth.session_ttl").unwrap();
    assert_eq!(
        view.source,
        Source::Database {
            version: 3,
            by: Some(7),
            at: 1_700_000_000_000
        }
    );
    assert_eq!(view.version, 3);
    assert_eq!(view.value, json!("60d"));
    assert_eq!(view.default, json!("30d"));
    assert_eq!(
        s.view("auth.session_idle_ttl").unwrap().source,
        Source::Default
    );
}

#[test]
fn invalid_or_unknown_stored_rows_fall_back_with_warnings() {
    let rows = vec![
        stored("auth.session_ttl", json!("1s"), 2),
        stored("legacy.removed", json!(true), 1),
    ];

    let (s, warnings) = Snapshot::build(&PinSet::empty(), rows);

    assert_eq!(s.view("auth.session_ttl").unwrap().source, Source::Default);
    let keys: Vec<&str> = warnings.iter().map(|w| w.key.as_str()).collect();
    assert_eq!(keys, ["auth.session_ttl", "legacy.removed"]);
}

#[test]
fn values_are_coerced_from_native_or_text_json() {
    let accepted = prepare(
        &PinSet::empty(),
        &[
            change("auth.lockout.max_failures", json!(7)),
            change("auth.ip_rate_limit.per_minute", json!("30")),
            change("devices.verify_uuid", json!("false")),
            change("signal.relay_servers", json!(["r.example.com:21117"])),
        ],
    )
    .unwrap();
    assert_eq!(accepted.len(), 4);
}

#[test]
fn range_enum_and_type_violations_are_rejected() {
    let rejection = prepare(
        &PinSet::empty(),
        &[
            change("auth.password.min_length", json!(4)),
            change("auth.password.min_length", json!(12)),
            change("auth.registration", json!("everyone")),
            change("auth.session_ttl", json!(3600)),
            change("smtp.port", json!(0)),
            change("signal.relay_servers", json!("host-without-port")),
            change("no.such.key", json!(1)),
        ],
    )
    .unwrap_err();

    assert_eq!(
        invalid_keys(rejection),
        [
            "auth.password.min_length",
            "auth.password.min_length",
            "auth.registration",
            "auth.session_ttl",
            "smtp.port",
            "signal.relay_servers",
            "no.such.key",
        ]
    );
}

#[test]
fn problems_never_echo_submitted_values() {
    let rejection = prepare(
        &PinSet::empty(),
        &[
            change("auth.registration", json!("xyzzy-registration")),
            change("smtp.host", json!("bad\u{7}host-plugh")),
        ],
    )
    .unwrap_err();

    let text = format!("{rejection} {rejection:?}");
    assert!(!text.contains("xyzzy"), "{text}");
    assert!(!text.contains("plugh"), "{text}");
}

#[test]
fn retention_accepts_zero_as_forever() {
    let accepted = prepare(
        &PinSet::empty(),
        &[change("audit.retention.login", json!("0"))],
    );
    assert!(accepted.is_ok(), "{accepted:?}");
}

#[test]
fn email_code_requires_smtp() {
    let base = Snapshot::defaults();

    let rejection =
        candidate(&base, &[change("auth.email_code.enabled", json!(true))]).unwrap_err();
    assert_eq!(invalid_keys(rejection), ["auth.email_code.enabled"]);

    let ok = candidate(
        &base,
        &[
            change("auth.email_code.enabled", json!(true)),
            change("smtp.host", json!("mail.example.com")),
            change("smtp.from", json!("rd@example.com")),
        ],
    )
    .unwrap();
    assert!(ok.get(&settings::AUTH_EMAIL_CODE_ENABLED));
}

#[test]
fn rules_are_checked_only_when_one_of_their_keys_changes() {
    let (pinned_bad, _) = Snapshot::build(
        &PinSet::empty(),
        vec![stored("auth.email_code.enabled", json!(true), 1)],
    );

    assert!(candidate(&pinned_bad, &[change("auth.lockout.window", json!("30m"))]).is_ok());
    assert!(
        candidate(
            &pinned_bad,
            &[change("smtp.host", json!("mail.example.com"))]
        )
        .is_err()
    );
}

#[test]
fn idle_ttl_must_not_exceed_session_ttl() {
    let base = Snapshot::defaults();
    let rejection = candidate(&base, &[change("auth.session_idle_ttl", json!("60d"))]).unwrap_err();
    assert_eq!(invalid_keys(rejection), ["auth.session_idle_ttl"]);
}

#[test]
fn sensitive_values_are_reported_as_set_only() {
    let base = Snapshot::defaults();
    let next = candidate(&base, &[change("smtp.password", json!("hunter2-0042"))]).unwrap();

    let view = next.view("smtp.password").unwrap();
    assert_eq!(view.value, json!({ "set": true }));
    assert_eq!(view.default, json!({ "set": false }));
    assert!(view.sensitive);
    let all = serde_json::to_string(&next.views()).unwrap();
    assert!(!all.contains("hunter2"), "{all}");
    assert!(!format!("{next:?}").contains("hunter2"));
}
