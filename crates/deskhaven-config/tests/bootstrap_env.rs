//! Acceptance tests: environment overrides, `_FILE` indirection, env warnings.

// Test code: panicking helpers are fine; figment::Jail fixes the closure error type.
#![allow(clippy::unwrap_used, clippy::panic, clippy::result_large_err)]

mod common;

use std::net::SocketAddr;

use common::jail_err;
use deskhaven_config::Source;
use deskhaven_config::bootstrap::{ConfigError, Warning};
use figment::Jail;

#[test]
fn env_only_nested_key_applies_when_absent_from_file() {
    Jail::expect_with(|jail| {
        jail.create_file("deskhaven.toml", "data_dir = \"/srv/dh\"\n")?;
        jail.clear_env();
        jail.set_env("DESKHAVEN_LISTEN__ADMIN", "127.0.0.1:21113");

        let loaded = common::load(jail).map_err(jail_err)?;

        let expected: SocketAddr = "127.0.0.1:21113".parse().map_err(|_| "addr")?;
        assert_eq!(loaded.config.listen.admin, Some(expected));
        assert_eq!(
            loaded.source("listen.admin"),
            Some(&Source::Env {
                var: "DESKHAVEN_LISTEN__ADMIN".into()
            })
        );
        Ok(())
    });
}

#[test]
fn env_overrides_file_value() {
    Jail::expect_with(|jail| {
        jail.create_file("deskhaven.toml", "[database]\nmax_connections = 3\n")?;
        jail.clear_env();
        jail.set_env("DESKHAVEN_DATABASE__MAX_CONNECTIONS", "7");

        let loaded = common::load(jail).map_err(jail_err)?;

        assert_eq!(loaded.config.database.max_connections, 7);
        Ok(())
    });
}

#[test]
fn file_indirection_reads_secret_from_file() {
    Jail::expect_with(|jail| {
        jail.create_file("mk", "c2VjcmV0LWtleQ==\n")?;
        let path = jail.directory().join("mk");
        jail.clear_env();
        jail.set_env("DESKHAVEN_SECURITY__MASTER_KEY_FILE", path.display());

        let loaded = common::load(jail).map_err(jail_err)?;

        assert_eq!(
            loaded.config.security.master_key.expose(),
            "c2VjcmV0LWtleQ=="
        );
        assert_eq!(
            loaded.source("security.master_key"),
            Some(&Source::Env {
                var: "DESKHAVEN_SECURITY__MASTER_KEY_FILE".into()
            })
        );
        Ok(())
    });
}

#[test]
fn file_indirection_conflicts_with_direct_value() {
    Jail::expect_with(|jail| {
        jail.create_file("mk", "abc")?;
        let path = jail.directory().join("mk");
        jail.clear_env();
        jail.set_env("DESKHAVEN_SECURITY__MASTER_KEY", "direct");
        jail.set_env("DESKHAVEN_SECURITY__MASTER_KEY_FILE", path.display());

        let err = common::load(jail).err().ok_or("expected conflict error")?;

        assert!(matches!(err, ConfigError::EnvConflict { .. }), "{err}");
        let msg = err.to_string();
        assert!(msg.contains("DESKHAVEN_SECURITY__MASTER_KEY_FILE"), "{msg}");
        Ok(())
    });
}

#[test]
fn file_indirection_missing_file_is_error() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        jail.set_env(
            "DESKHAVEN_DATABASE__URL_FILE",
            "/nonexistent/deskhaven/db-url",
        );

        let err = common::load(jail).err().ok_or("expected read error")?;

        assert!(matches!(err, ConfigError::EnvFileRead { .. }), "{err}");
        assert!(err.to_string().contains("/nonexistent/deskhaven/db-url"));
        Ok(())
    });
}

#[test]
fn key_ending_in_file_is_a_plain_key() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        jail.set_env("DESKHAVEN_SIGNAL__KEY_FILE", "/keys/id_ed25519");

        let loaded = common::load(jail).map_err(jail_err)?;

        assert_eq!(
            loaded.config.signal.key_file.to_str(),
            Some("/keys/id_ed25519")
        );
        Ok(())
    });
}

#[test]
fn unrecognized_env_var_produces_warning_not_error() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        jail.set_env("DESKHAVEN_BOGUS", "1");
        jail.set_env("DESKHAVEN_LISTEN__NOPE", "1");
        jail.set_env("DESKHAVEN_CONFIG_DIR", "/x");

        let loaded = common::load(jail).map_err(jail_err)?;

        for var in [
            "DESKHAVEN_BOGUS",
            "DESKHAVEN_LISTEN__NOPE",
            "DESKHAVEN_CONFIG_DIR",
        ] {
            assert!(
                loaded
                    .warnings
                    .contains(&Warning::UnknownEnv { var: var.into() }),
                "missing warning for {var}: {:?}",
                loaded.warnings
            );
        }
        Ok(())
    });
}

#[test]
fn env_pin_lands_in_settings_map_with_source() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        jail.set_env("DESKHAVEN_SETTINGS__AUTH__SESSION_TTL", "1d");

        let loaded = common::load(jail).map_err(jail_err)?;

        let pin = loaded
            .config
            .settings
            .get("auth.session_ttl")
            .map(|p| p.value());
        assert_eq!(pin, Some(&serde_json::json!("1d")));
        assert_eq!(
            loaded.source("settings.auth.session_ttl"),
            Some(&Source::Env {
                var: "DESKHAVEN_SETTINGS__AUTH__SESSION_TTL".into()
            })
        );
        Ok(())
    });
}

#[test]
fn clean_environment_has_no_warnings() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        let loaded = common::load(jail).map_err(jail_err)?;
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        Ok(())
    });
}
