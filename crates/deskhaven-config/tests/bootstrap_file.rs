//! Acceptance tests: file lookup, unknown keys, precedence, pins.

// Test code: panicking helpers are fine; figment::Jail fixes the closure error type.
#![allow(clippy::unwrap_used, clippy::panic, clippy::result_large_err)]

mod common;

use common::jail_err;
use deskhaven_config::Source;
use deskhaven_config::bootstrap::{Bootstrap, CliOverrides, ConfigError, LogFormat, load};
use figment::Jail;

#[test]
fn missing_file_uses_defaults() {
    Jail::expect_with(|jail| {
        jail.clear_env();

        let loaded = common::load(jail).map_err(jail_err)?;

        assert_eq!(loaded.config, Bootstrap::default());
        assert_eq!(loaded.config_file, None);
        assert_eq!(loaded.source("listen.client_api"), Some(&Source::Default));
        Ok(())
    });
}

#[test]
fn unknown_top_level_key_names_key_and_file() {
    Jail::expect_with(|jail| {
        jail.create_file("deskhaven.toml", "bogus_key = 1\n")?;
        jail.clear_env();

        let err = common::load(jail)
            .err()
            .ok_or("expected unknown key error")?;

        let msg = err.to_string();
        assert!(matches!(err, ConfigError::Extract(_)), "{msg}");
        assert!(msg.contains("bogus_key"), "{msg}");
        assert!(msg.contains("deskhaven.toml"), "{msg}");
        Ok(())
    });
}

#[test]
fn unknown_nested_key_names_its_path() {
    Jail::expect_with(|jail| {
        jail.create_file("deskhaven.toml", "[listen]\nclient_apii = \"0.0.0.0:1\"\n")?;
        jail.clear_env();

        let err = common::load(jail)
            .err()
            .ok_or("expected unknown key error")?;

        let msg = err.to_string();
        assert!(msg.contains("client_apii"), "{msg}");
        assert!(msg.contains("listen"), "{msg}");
        Ok(())
    });
}

#[test]
fn explicit_config_path_must_exist() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        let mut options = common::options(jail);
        options.config_path = Some(jail.directory().join("missing.toml"));

        let err = load(&options).err().ok_or("expected not found")?;

        assert!(
            matches!(err, ConfigError::ConfigFileNotFound { .. }),
            "{err}"
        );
        Ok(())
    });
}

#[test]
fn deskhaven_config_env_selects_file() {
    Jail::expect_with(|jail| {
        jail.create_file("custom.toml", "[log]\nformat = \"json\"\n")?;
        jail.clear_env();
        jail.set_env(
            "DESKHAVEN_CONFIG",
            jail.directory().join("custom.toml").display(),
        );

        let loaded = common::load(jail).map_err(jail_err)?;

        assert_eq!(loaded.config.log.format, LogFormat::Json);
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        Ok(())
    });
}

#[test]
fn cli_beats_env_beats_file() {
    Jail::expect_with(|jail| {
        jail.create_file(
            "deskhaven.toml",
            "data_dir = \"/from/file\"\n[log]\nlevel = \"warn\"\n",
        )?;
        jail.clear_env();
        jail.set_env("DESKHAVEN_DATA_DIR", "/from/env");
        jail.set_env("DESKHAVEN_LOG__LEVEL", "debug");
        let mut options = common::options(jail);
        options.cli = CliOverrides::default().with("data_dir", "/from/cli");

        let loaded = load(&options).map_err(jail_err)?;

        assert_eq!(loaded.config.data_dir.to_str(), Some("/from/cli"));
        assert_eq!(loaded.source("data_dir"), Some(&Source::Cli));
        assert_eq!(loaded.config.log.level, "debug");
        Ok(())
    });
}

#[test]
fn unknown_cli_key_is_error() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        let mut options = common::options(jail);
        options.cli = CliOverrides::default().with("nope.key", "1");

        let err = load(&options).err().ok_or("expected unknown cli key")?;

        assert!(matches!(err, ConfigError::UnknownCliKey { .. }), "{err}");
        Ok(())
    });
}

#[test]
fn file_pins_accept_dotted_keys_and_env_overrides_them() {
    Jail::expect_with(|jail| {
        let toml =
            "[settings]\n\"auth.session_ttl\" = \"30d\"\n\"auth.lockout.max_failures\" = 3\n";
        jail.create_file("deskhaven.toml", toml)?;
        jail.clear_env();
        jail.set_env("DESKHAVEN_SETTINGS__AUTH__SESSION_TTL", "2d");

        let loaded = common::load(jail).map_err(jail_err)?;

        let settings = &loaded.config.settings;
        assert_eq!(
            settings.get("auth.session_ttl"),
            Some(&serde_json::json!("2d"))
        );
        assert_eq!(
            settings.get("auth.lockout.max_failures"),
            Some(&serde_json::json!(3))
        );
        let file = jail.directory().join("deskhaven.toml");
        assert_eq!(
            loaded.source("settings.auth.lockout.max_failures"),
            Some(&Source::File { path: file })
        );
        Ok(())
    });
}

#[test]
fn duplicate_pin_in_file_is_error() {
    Jail::expect_with(|jail| {
        let toml =
            "[settings]\n\"auth.session_ttl\" = \"30d\"\n[settings.auth]\nsession_ttl = \"1d\"\n";
        jail.create_file("deskhaven.toml", toml)?;
        jail.clear_env();

        let err = common::load(jail)
            .err()
            .ok_or("expected duplicate pin error")?;

        assert!(err.to_string().contains("auth.session_ttl"), "{err}");
        Ok(())
    });
}
