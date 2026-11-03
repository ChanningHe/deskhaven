//! Pins from the bootstrap `[settings]` table / `DESKHAVEN_SETTINGS__*` env.

// Test code: panicking helpers are fine; figment::Jail fixes the closure error type.
#![allow(clippy::unwrap_used, clippy::panic, clippy::result_large_err)]

mod common;

use common::jail_err;
use deskhaven_config::Source;
use deskhaven_config::settings::{
    self, Change, HostPort, PinSet, Registration, Rejection, Reset, Snapshot, TfaEnforce, prepare,
    prepare_batch,
};
use figment::Jail;

fn pins(jail: &Jail) -> Result<PinSet, String> {
    let loaded = common::load(jail).map_err(|e| e.to_string())?;
    PinSet::from_bootstrap(&loaded.config.settings).map_err(|e| e.to_string())
}

fn env_source(var: &str) -> Source {
    Source::Env { var: var.into() }
}

#[test]
fn env_text_pins_coerce_per_kind() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        jail.set_env("DESKHAVEN_SETTINGS__AUTH__SESSION_TTL", "7d");
        jail.set_env("DESKHAVEN_SETTINGS__AUTH__EMAIL_CODE__ENABLED", "true");
        jail.set_env("DESKHAVEN_SETTINGS__AUTH__PASSWORD__MIN_LENGTH", "010");
        jail.set_env("DESKHAVEN_SETTINGS__AUTH__REGISTRATION", "invite");
        jail.set_env("DESKHAVEN_SETTINGS__SIGNAL__RELAY_SERVERS", "a:1, b:2");
        jail.set_env("DESKHAVEN_SETTINGS__SMTP__HOST", "mail.example.com");
        jail.set_env("DESKHAVEN_SETTINGS__SMTP__FROM", "rd@example.com");

        let pins = pins(jail)?;
        let (s, warnings) = Snapshot::build(&pins, Vec::new());

        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            s.get(&settings::AUTH_SESSION_TTL).as_std().as_secs(),
            7 * 86_400
        );
        assert!(s.get(&settings::AUTH_EMAIL_CODE_ENABLED));
        assert_eq!(s.get(&settings::AUTH_PASSWORD_MIN_LENGTH), 10);
        assert_eq!(s.get(&settings::AUTH_REGISTRATION), Registration::Invite);
        let relays: Vec<String> = s
            .get(&settings::SIGNAL_RELAY_SERVERS)
            .iter()
            .map(HostPort::to_string)
            .collect();
        assert_eq!(relays, ["a:1", "b:2"]);

        let view = s.view("auth.session_ttl").unwrap();
        assert!(view.pinned);
        assert_eq!(
            view.source,
            env_source("DESKHAVEN_SETTINGS__AUTH__SESSION_TTL")
        );
        assert!(pins.is_pinned("auth.session_ttl"));
        assert!(!pins.is_pinned("auth.session_idle_ttl"));
        Ok(())
    });
}

#[test]
fn toml_pins_keep_native_types() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        let file = jail.directory().join("deskhaven.toml");
        jail.create_file(
            "deskhaven.toml",
            "[settings]\n\
             \"auth.lockout.max_failures\" = 3\n\
             \"devices.verify_uuid\" = false\n\
             \"auth.tfa.enforce\" = \"admins\"\n\
             \"signal.relay_servers\" = [\"r1.example.com:21117\", \"[::1]:21117\"]\n",
        )?;

        let pins = pins(jail)?;
        let (s, _) = Snapshot::build(&pins, Vec::new());

        assert_eq!(s.get(&settings::AUTH_LOCKOUT_MAX_FAILURES), 3);
        assert!(!s.get(&settings::DEVICES_VERIFY_UUID));
        assert_eq!(s.get(&settings::AUTH_TFA_ENFORCE), TfaEnforce::Admins);
        let relays = s.get(&settings::SIGNAL_RELAY_SERVERS);
        assert_eq!(relays.len(), 2);
        assert_eq!(relays[1].host(), "::1");
        assert_eq!(relays[1].to_string(), "[::1]:21117");
        assert_eq!(
            s.view("auth.lockout.max_failures").unwrap().source,
            Source::File { path: file }
        );
        Ok(())
    });
}

#[test]
fn file_indirection_pins_a_secret() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        jail.create_file("smtp_pw", "s3cret-0042\n")?;
        let path = jail.directory().join("smtp_pw");
        jail.set_env(
            "DESKHAVEN_SETTINGS__SMTP__PASSWORD_FILE",
            path.display().to_string(),
        );

        let pins = pins(jail)?;
        let (s, _) = Snapshot::build(&pins, Vec::new());

        assert_eq!(s.get(&settings::SMTP_PASSWORD).expose(), "s3cret-0042");
        let view = s.view("smtp.password").unwrap();
        assert_eq!(view.value, serde_json::json!({ "set": true }));
        assert!(!format!("{s:?}").contains("s3cret"));
        Ok(())
    });
}

#[test]
fn invalid_pin_fails_without_echoing_the_value() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        jail.set_env("DESKHAVEN_SETTINGS__AUTH__SESSION_TTL", "forever-xyzzy");
        jail.set_env("DESKHAVEN_SETTINGS__AUTH__REGISTRATION", "plugh");
        jail.set_env("DESKHAVEN_SETTINGS__SMTP__PORT", "70000");

        let err = pins(jail).unwrap_err();

        for needle in [
            "auth.session_ttl",
            "DESKHAVEN_SETTINGS__AUTH__SESSION_TTL",
            "auth.registration",
            "smtp.port",
        ] {
            assert!(err.contains(needle), "missing {needle}: {err}");
        }
        for secret in ["forever-xyzzy", "plugh", "70000"] {
            assert!(!err.contains(secret), "echoed {secret}: {err}");
        }
        Ok(())
    });
}

#[test]
fn out_of_range_toml_pin_names_the_file() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        jail.create_file(
            "deskhaven.toml",
            "[settings]\n\"auth.session_ttl\" = \"1s\"\n",
        )?;

        let err = pins(jail).unwrap_err();

        assert!(err.contains("auth.session_ttl"), "{err}");
        assert!(err.contains("deskhaven.toml"), "{err}");
        assert!(err.contains("between"), "{err}");
        Ok(())
    });
}

#[test]
fn unknown_pin_keys_are_listed() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        jail.set_env("DESKHAVEN_SETTINGS__AUTH__SESION_TTL", "1d");
        jail.create_file("deskhaven.toml", "[settings]\n\"smtp.hots\" = \"x\"\n")?;

        let err = pins(jail).unwrap_err();

        assert!(err.contains("auth.sesion_ttl"), "{err}");
        assert!(err.contains("smtp.hots"), "{err}");
        assert!(err.contains("unknown"), "{err}");
        Ok(())
    });
}

#[test]
fn unknown_pin_keys_are_sanitized_in_errors() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        let long = "k".repeat(200);
        jail.create_file(
            "deskhaven.toml",
            &format!("[settings]\n\"smtp.ho\\u0007st\" = \"x\"\n\"{long}.x\" = 1\n"),
        )?;

        let err = pins(jail).unwrap_err();

        assert!(err.contains("smtp.ho\\u{7}st"), "{err}");
        assert!(!err.contains('\u{7}'), "{err}");
        assert!(!err.contains(&long), "{err}");
        Ok(())
    });
}

#[test]
fn updates_to_pinned_keys_are_rejected() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        jail.set_env("DESKHAVEN_SETTINGS__AUTH__SESSION_TTL", "7d");
        let pins = pins(jail)?;

        let change = |key: &str| Change {
            key: key.to_owned(),
            value: serde_json::json!("1d"),
            version: 0,
        };
        let rejection = prepare(
            &pins,
            &[change("auth.lockout.window"), change("auth.session_ttl")],
        )
        .unwrap_err();

        assert_eq!(
            rejection,
            Rejection::Pinned {
                keys: vec!["auth.session_ttl".to_owned()]
            }
        );
        Ok(())
    });
}

#[test]
fn resets_of_pinned_keys_are_rejected() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        jail.set_env("DESKHAVEN_SETTINGS__AUTH__SESSION_TTL", "7d");
        let pins = pins(jail)?;

        let reset = |key: &str| Reset {
            key: key.to_owned(),
            version: 0,
        };
        let rejection = prepare_batch(
            &pins,
            &[],
            &[reset("auth.lockout.window"), reset("auth.session_ttl")],
        )
        .unwrap_err();

        assert_eq!(
            rejection,
            Rejection::Pinned {
                keys: vec!["auth.session_ttl".to_owned()]
            }
        );
        Ok(())
    });
}

#[test]
fn no_pins_is_empty() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        let loaded = common::load(jail).map_err(jail_err)?;
        let pins = PinSet::from_bootstrap(&loaded.config.settings)
            .map_err(|e| figment::Error::from(e.to_string()))?;
        assert!(pins.is_empty());
        Ok(())
    });
}
