//! Review-driven tests: schema-aware typing, secret hygiene, pin `_FILE`,
//! list syntax, log filter validation.

// Test code: panicking helpers are fine; figment::Jail fixes the closure error type.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::result_large_err
)]

mod common;

use common::jail_err;
use deskhaven_config::Source;
use deskhaven_config::bootstrap::{CliOverrides, ConfigError, Loaded, load};
use figment::Jail;
use serde_json::json;

fn load_error(jail: &Jail) -> ConfigError {
    common::load(jail).expect_err("expected a load error")
}

fn pin<'a>(loaded: &'a Loaded, key: &str) -> (&'a serde_json::Value, &'a Source) {
    let pin = loaded
        .config
        .settings
        .get(key)
        .unwrap_or_else(|| panic!("no pin {key}"));
    (pin.value(), pin.source())
}

#[test]
fn number_and_bool_looking_env_strings_keep_string_type() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        jail.set_env("DESKHAVEN_SECURITY__MASTER_KEY", "12345678");
        jail.set_env("DESKHAVEN_DATA_DIR", "12345");
        jail.set_env("DESKHAVEN_SIGNAL__KEY_FILE", "true");

        let loaded = common::load(jail).map_err(jail_err)?;

        assert_eq!(loaded.config.security.master_key.expose(), "12345678");
        assert_eq!(loaded.config.data_dir.to_str(), Some("12345"));
        assert_eq!(loaded.config.signal.key_file.to_str(), Some("true"));
        Ok(())
    });
}

#[test]
fn number_looking_file_indirection_keeps_leading_zero() {
    Jail::expect_with(|jail| {
        jail.create_file("mk", "0123\n")?;
        jail.clear_env();
        jail.set_env(
            "DESKHAVEN_SECURITY__MASTER_KEY_FILE",
            jail.directory().join("mk").display(),
        );

        let loaded = common::load(jail).map_err(jail_err)?;

        assert_eq!(loaded.config.security.master_key.expose(), "0123");
        Ok(())
    });
}

#[test]
fn secret_type_error_in_file_never_echoes_value() {
    Jail::expect_with(|jail| {
        jail.create_file("deskhaven.toml", "[security]\nmaster_key = 98765432\n")?;
        jail.clear_env();

        let msg = load_error(jail).to_string();

        assert!(msg.contains("master_key"), "{msg}");
        assert!(!msg.contains("98765432"), "{msg}");
        Ok(())
    });
}

#[test]
fn non_secret_type_error_names_var_and_expected_type_only() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        jail.set_env("DESKHAVEN_SIGNAL__PORT", "abc");

        let msg = load_error(jail).to_string();

        assert!(
            !msg.contains("abc") && msg.contains("DESKHAVEN_SIGNAL__PORT") && msg.contains("u16"),
            "{msg}"
        );
        Ok(())
    });
}

#[test]
fn pin_file_indirection_reads_value_into_base_key() {
    Jail::expect_with(|jail| {
        jail.create_file("smtp", "0042pass\n")?;
        jail.clear_env();
        let var = "DESKHAVEN_SETTINGS__SMTP__PASSWORD_FILE";
        jail.set_env(var, jail.directory().join("smtp").display());

        let loaded = common::load(jail).map_err(jail_err)?;

        assert_eq!(
            pin(&loaded, "smtp.password"),
            (&json!("0042pass"), &Source::Env { var: var.into() })
        );
        assert!(loaded.config.settings.get("smtp.password_file").is_none());
        Ok(())
    });
}

#[test]
fn pin_sources_cover_file_env_and_cli() {
    Jail::expect_with(|jail| {
        jail.create_file("deskhaven.toml", "[settings]\n\"a.from_file\" = 1\n")?;
        jail.clear_env();
        jail.set_env("DESKHAVEN_SETTINGS__A__FROM_ENV", "2");
        let mut options = common::options(jail);
        options.cli = CliOverrides::default().with("settings.a.from_cli", "3");

        let loaded = load(&options).map_err(jail_err)?;

        let file = jail.directory().join("deskhaven.toml");
        assert_eq!(
            pin(&loaded, "a.from_file"),
            (&json!(1), &Source::File { path: file })
        );
        let env = Source::Env {
            var: "DESKHAVEN_SETTINGS__A__FROM_ENV".into(),
        };
        assert_eq!(pin(&loaded, "a.from_env"), (&json!("2"), &env));
        assert_eq!(pin(&loaded, "a.from_cli"), (&json!("3"), &Source::Cli));
        Ok(())
    });
}

#[test]
fn pin_shadowing_is_error() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        jail.set_env("DESKHAVEN_SETTINGS__A", "1");
        jail.set_env("DESKHAVEN_SETTINGS__A__B", "2");

        let msg = load_error(jail).to_string();

        assert!(msg.contains("settings.a"), "{msg}");
        Ok(())
    });
}

#[test]
fn pins_are_redacted_in_debug_and_serialization() {
    Jail::expect_with(|jail| {
        jail.create_file(
            "deskhaven.toml",
            "[settings]\n\"smtp.password\" = \"pin-secret\"\n",
        )?;
        jail.clear_env();

        let loaded = common::load(jail).map_err(jail_err)?;

        let debug = format!("{loaded:?}");
        let json = serde_json::to_string(&loaded.config).unwrap();
        for rendered in [&debug, &json] {
            assert!(!rendered.contains("pin-secret"), "{rendered}");
            assert!(rendered.contains("smtp.password"), "{rendered}");
        }
        Ok(())
    });
}

#[test]
fn list_keys_accept_comma_separated_and_array_syntax() {
    for raw in ["10.0.0.0/8, 192.168.1.1", "[10.0.0.0/8, 192.168.1.1]"] {
        Jail::expect_with(|jail| {
            jail.clear_env();
            jail.set_env("DESKHAVEN_SECURITY__TRUSTED_PROXIES", raw);

            let loaded = common::load(jail).map_err(jail_err)?;

            let proxies: Vec<String> = loaded
                .config
                .security
                .trusted_proxies
                .iter()
                .map(ToString::to_string)
                .collect();
            assert_eq!(proxies, ["10.0.0.0/8", "192.168.1.1/32"], "{raw}");
            Ok(())
        });
    }
}

#[test]
fn empty_config_env_is_treated_as_unset() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        jail.set_env("DESKHAVEN_CONFIG", "");

        let loaded = common::load(jail).map_err(jail_err)?;

        assert_eq!(loaded.config_file, None);
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        Ok(())
    });
}

#[test]
fn invalid_log_filter_is_rejected() {
    Jail::expect_with(|jail| {
        jail.create_file("deskhaven.toml", "[log]\nlevel = \"foo=notalevel\"\n")?;
        jail.clear_env();

        let err = load_error(jail);

        assert!(matches!(err, ConfigError::Invalid { .. }), "{err}");
        assert!(err.to_string().contains("log.level"), "{err}");
        Ok(())
    });
}

#[test]
fn url_errors_never_echo_credentials() {
    Jail::expect_with(|jail| {
        let toml = "public_url = \"https://user:hunter2@rd.example.com\"\n";
        jail.create_file("deskhaven.toml", toml)?;
        jail.clear_env();

        let msg = load_error(jail).to_string();

        assert!(
            msg.contains("public_url") && !msg.contains("hunter2"),
            "{msg}"
        );
        Ok(())
    });
}
