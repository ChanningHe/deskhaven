//! Pure update validation and snapshot building (no database).

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::panic)]

use deskhaven_config::Source;
use deskhaven_config::settings::{
    self, Change, PinSet, Rejection, Snapshot, Stored, check_rules, prepare,
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
        value: Some(value),
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
    let next = base.with_accepted(&accepted, None, 0);
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

    let ttl = s.view("auth.session_ttl").unwrap();
    assert_eq!(ttl.source, Source::Default);
    assert_eq!(ttl.value, json!("30d"));
    assert_eq!(ttl.version, 2, "kept so the row can be overwritten");
    let mut keys: Vec<&str> = warnings.iter().map(|w| w.key.as_str()).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["auth.session_ttl", "legacy.removed"]);
}

#[test]
fn unreadable_rows_keep_their_version_without_another_warning() {
    let row = Stored {
        value: None,
        ..stored("smtp.password", json!(null), 4)
    };

    let (s, warnings) = Snapshot::build(&PinSet::empty(), vec![row]);

    assert!(warnings.is_empty(), "{warnings:?}");
    let view = s.view("smtp.password").unwrap();
    assert_eq!((view.source, view.version), (Source::Default, 4));
    assert_eq!(view.value, json!({ "set": false }));
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
fn unknown_keys_are_echoed_truncated_and_escaped() {
    let long = format!("x\n{}", "k".repeat(200));
    let rejection = prepare(&PinSet::empty(), &[change(&long, json!(1))]).unwrap_err();

    let keys = invalid_keys(rejection);
    assert!(keys[0].starts_with("x\\n"), "{keys:?}");
    assert!(keys[0].chars().count() < 80, "{keys:?}");
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
fn lockout_window_must_not_exceed_four_durations() {
    let base = Snapshot::defaults();
    assert_eq!(settings::LOCKOUT_GLOBAL_FACTOR, 4);

    let at_limit = candidate(&base, &[change("auth.lockout.window", json!("1h"))]);
    assert!(at_limit.is_ok(), "{at_limit:?}");
    let too_long = candidate(&base, &[change("auth.lockout.window", json!("61m"))]).unwrap_err();
    assert_eq!(invalid_keys(too_long), ["auth.lockout.window"]);

    let short_duration =
        candidate(&base, &[change("auth.lockout.duration", json!("3m"))]).unwrap_err();
    assert_eq!(invalid_keys(short_duration), ["auth.lockout.window"]);
    let both = candidate(
        &base,
        &[
            change("auth.lockout.duration", json!("1m")),
            change("auth.lockout.window", json!("4m")),
        ],
    );
    assert!(both.is_ok(), "{both:?}");
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

fn accepted_json(key: &str, value: Json) -> Result<Json, Rejection> {
    let accepted = prepare(&PinSet::empty(), &[change(key, value)])?;
    Ok(accepted[0].value().to_json())
}

#[test]
fn enums_are_case_insensitive_and_canonical() {
    assert_eq!(
        accepted_json("auth.registration", json!("Invite")),
        Ok(json!("invite"))
    );
    assert_eq!(
        accepted_json("smtp.tls", json!(" STARTTLS ")),
        Ok(json!("starttls"))
    );
    assert!(accepted_json("auth.registration", json!("invites")).is_err());
}

#[test]
fn u32_text_must_be_plain_digits() {
    assert_eq!(accepted_json("smtp.port", json!("0465")), Ok(json!(465)));
    for bad in ["+465", "-1", "4 65", ""] {
        assert!(accepted_json("smtp.port", json!(bad)).is_err(), "{bad:?}");
    }
}

#[test]
fn comma_lists_reject_empty_items_like_arrays() {
    assert_eq!(
        accepted_json("signal.relay_servers", json!("")),
        Ok(json!([]))
    );
    assert_eq!(
        accepted_json("signal.relay_servers", json!(" a:1 , b:2 ")),
        Ok(json!(["a:1", "b:2"]))
    );
    for bad in [json!("a:1,,b:2"), json!("a:1,"), json!(["a:1", ""])] {
        assert!(
            accepted_json("signal.relay_servers", bad.clone()).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn relays_reject_unspecified_and_broadcast_addresses() {
    for bad in ["0.0.0.0:21117", "255.255.255.255:21117", "[::]:21117"] {
        assert!(
            accepted_json("signal.relay_servers", json!([bad])).is_err(),
            "{bad}"
        );
    }
    assert!(accepted_json("signal.relay_servers", json!(["10.0.0.1:21117"])).is_ok());
}

#[test]
fn smtp_host_must_be_a_hostname_or_ip() {
    for ok in ["", "mail.example.com", "10.0.0.25", "2001:db8::25"] {
        assert!(accepted_json("smtp.host", json!(ok)).is_ok(), "{ok:?}");
    }
    for bad in [
        "mail example.com",
        "mail..example.com",
        "-mail.example.com",
        "mail.example.com:25",
    ] {
        assert!(accepted_json("smtp.host", json!(bad)).is_err(), "{bad:?}");
    }
}

#[test]
fn smtp_from_must_look_like_an_address() {
    for ok in ["", "rd@example.com", "Desk.Haven+ops@mail.example.org"] {
        assert!(accepted_json("smtp.from", json!(ok)).is_ok(), "{ok:?}");
    }
    for bad in [
        "rd",
        "@example.com",
        "rd@",
        "a@b@c",
        "rd @example.com",
        "rd@exa mple.com",
    ] {
        assert!(accepted_json("smtp.from", json!(bad)).is_err(), "{bad:?}");
    }
}

#[test]
fn security_relevant_settings_fall_back_strictly_on_bad_rows() {
    let bad = |key: &str| stored(key, json!("garbage-value"), 2);
    let unreadable = |key: &str| Stored {
        value: None,
        ..stored(key, json!(null), 3)
    };
    let rows = vec![
        bad("auth.tfa.enforce"),
        bad("signal.acl.default"),
        unreadable("signal.acl.on_error"),
        bad("auth.registration"),
        bad("devices.verify_uuid"),
        bad("auth.session_ttl"),
    ];

    let (s, _) = Snapshot::build(&PinSet::empty(), rows);

    assert_eq!(
        s.get(&settings::AUTH_TFA_ENFORCE),
        settings::TfaEnforce::Admins
    );
    assert_eq!(
        s.get(&settings::SIGNAL_ACL_DEFAULT),
        settings::AclAction::Deny
    );
    assert_eq!(
        s.get(&settings::SIGNAL_ACL_ON_ERROR),
        settings::AclAction::Deny
    );
    assert_eq!(
        s.get(&settings::AUTH_REGISTRATION),
        settings::Registration::Disabled
    );
    assert!(s.get(&settings::DEVICES_VERIFY_UUID));
    // No strict policy: plain default.
    assert_eq!(
        s.get(&settings::AUTH_SESSION_TTL).as_std().as_secs(),
        30 * 86_400
    );
    let tfa = s.entry("auth.tfa.enforce").unwrap();
    assert!(tfa.is_fallback());
    assert_eq!((tfa.source(), tfa.version()), (&Source::Default, 2));
    // Without a bad row the declared default applies.
    assert_eq!(
        Snapshot::defaults().get(&settings::AUTH_TFA_ENFORCE),
        settings::TfaEnforce::Off
    );
}
