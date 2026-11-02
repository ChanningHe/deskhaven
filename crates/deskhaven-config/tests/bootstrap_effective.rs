//! Acceptance tests: effective view (redaction + provenance), validation, schema.

// Test code: panicking helpers are fine; figment::Jail fixes the closure error type.
#![allow(clippy::unwrap_used, clippy::panic, clippy::result_large_err)]

mod common;

use common::jail_err;
use deskhaven_config::Source;
use deskhaven_config::bootstrap::{ConfigError, EffectiveEntry, Warning, json_schema};
use figment::Jail;
use serde_json::json;

fn entry<'a>(entries: &'a [EffectiveEntry], key: &str) -> &'a EffectiveEntry {
    entries
        .iter()
        .find(|e| e.key == key)
        .unwrap_or_else(|| panic!("no entry {key}"))
}

#[test]
fn effective_redacts_secrets_and_reports_sources() {
    Jail::expect_with(|jail| {
        jail.create_file(
            "deskhaven.toml",
            "[database]\nurl = \"postgres://u:pw@db/dh\"\n",
        )?;
        jail.clear_env();
        jail.set_env("DESKHAVEN_SECURITY__MASTER_KEY", "super-secret");

        let loaded = common::load(jail).map_err(jail_err)?;
        let entries = loaded.effective();

        let file = jail.directory().join("deskhaven.toml");
        let db = entry(entries, "database.url");
        assert_eq!(db.value, json!("<redacted>"));
        assert_eq!(db.source, Source::File { path: file });
        let mk = entry(entries, "security.master_key");
        assert_eq!(mk.value, json!("<redacted>"));
        assert_eq!(
            mk.source,
            Source::Env {
                var: "DESKHAVEN_SECURITY__MASTER_KEY".into()
            }
        );
        let port = entry(entries, "signal.port");
        assert_eq!(port.value, json!(21116));
        assert_eq!(port.source, Source::Default);

        let rendered = format!("{entries:?}");
        assert!(
            !rendered.contains("super-secret") && !rendered.contains("pw@"),
            "{rendered}"
        );
        Ok(())
    });
}

#[test]
fn effective_lists_every_schema_leaf() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        let loaded = common::load(jail).map_err(jail_err)?;
        let keys: Vec<String> = loaded.effective().iter().map(|e| e.key.clone()).collect();

        for key in [
            "data_dir",
            "public_url",
            "listen.client_api",
            "listen.admin",
            "listen.admin_public_url",
            "signal.port",
            "signal.key_file",
            "signal.enabled",
            "database.url",
            "database.max_connections",
            "security.master_key",
            "security.trusted_proxies",
            "log.level",
            "log.format",
        ] {
            assert!(keys.iter().any(|k| k == key), "missing {key} in {keys:?}");
        }
        Ok(())
    });
}

#[test]
fn source_display_matches_cli_contract() {
    assert_eq!(Source::Default.to_string(), "default");
    assert_eq!(Source::Cli.to_string(), "cli");
    assert_eq!(
        Source::Env {
            var: "DESKHAVEN_X".into()
        }
        .to_string(),
        "env:DESKHAVEN_X"
    );
    let file = Source::File {
        path: "/etc/deskhaven/deskhaven.toml".into(),
    };
    assert_eq!(file.to_string(), "file:/etc/deskhaven/deskhaven.toml");
}

/// Loads `toml` as the config file and returns the resulting error.
fn load_error(toml: &str) -> ConfigError {
    let mut out = None;
    Jail::expect_with(|jail| {
        jail.create_file("deskhaven.toml", toml)?;
        jail.clear_env();
        out = common::load(jail).err();
        Ok(())
    });
    out.unwrap_or_else(|| panic!("config unexpectedly valid: {toml}"))
}

fn assert_invalid(toml: &str, needle: &str) {
    let err = load_error(toml);
    assert!(matches!(err, ConfigError::Invalid { .. }), "{err}");
    assert!(err.to_string().contains(needle), "{err}");
}

#[test]
fn rejects_malformed_public_url() {
    assert_invalid("public_url = \"ftp://rd.example.com\"\n", "public_url");
    assert_invalid(
        "public_url = \"https://rd.example.com/?q=1\"\n",
        "public_url",
    );
}

#[test]
fn rejects_port_conflict_between_api_and_signal() {
    assert_invalid("[listen]\nclient_api = \"0.0.0.0:21115\"\n", "21115");
}

#[test]
fn rejects_admin_port_overlapping_client_api() {
    let toml = "[listen]\nclient_api = \"0.0.0.0:9000\"\nadmin = \"127.0.0.1:9000\"\n";
    assert_invalid(toml, "9000");
}

#[test]
fn rejects_malformed_cidr_naming_value() {
    let err = load_error("[security]\ntrusted_proxies = [\"10.0.0.0/33\"]\n");
    assert!(matches!(err, ConfigError::Extract(_)), "{err}");
    assert!(err.to_string().contains("10.0.0.0/33"), "{err}");
}

#[test]
fn admin_equal_to_client_api_means_not_split() {
    Jail::expect_with(|jail| {
        let toml = "[listen]\nclient_api = \"0.0.0.0:9000\"\nadmin = \"0.0.0.0:9000\"\n";
        jail.create_file("deskhaven.toml", toml)?;
        jail.clear_env();

        let loaded = common::load(jail).map_err(jail_err)?;

        assert_eq!(loaded.config.listen.admin_addr(), None);
        Ok(())
    });
}

#[test]
fn split_admin_without_public_url_warns() {
    Jail::expect_with(|jail| {
        jail.create_file("deskhaven.toml", "[listen]\nadmin = \"127.0.0.1:21113\"\n")?;
        jail.clear_env();

        let loaded = common::load(jail).map_err(jail_err)?;

        assert!(
            loaded.warnings.contains(&Warning::AdminRedirectUnset),
            "{:?}",
            loaded.warnings
        );
        Ok(())
    });
}

#[test]
fn schema_forbids_unknown_keys_and_describes_sections() {
    let schema = json_schema();

    assert_eq!(schema["additionalProperties"], json!(false));
    assert_eq!(
        schema["properties"]["listen"]["$ref"],
        json!("#/$defs/Listen")
    );
    assert_eq!(
        schema["$defs"]["Listen"]["additionalProperties"],
        json!(false)
    );
    assert_eq!(schema["properties"]["settings"]["type"], json!("object"));
    assert_eq!(schema["$defs"]["Secret"]["writeOnly"], json!(true));
}
