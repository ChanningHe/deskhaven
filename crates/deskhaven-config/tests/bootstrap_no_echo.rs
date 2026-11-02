//! Config errors never echo found values, for any key: only key path,
//! expected type and source. Every case plants `HUNTER2` and checks it
//! never appears in the error text.

// Test code: panicking helpers are fine; figment::Jail fixes the closure error type.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::result_large_err
)]

mod common;

use deskhaven_config::bootstrap::ConfigError;
use figment::Jail;

const PLANTED: &str = "HUNTER2";

fn file_error(toml: &str) -> ConfigError {
    let mut out = None;
    Jail::expect_with(|jail| {
        jail.create_file("deskhaven.toml", toml)?;
        jail.clear_env();
        out = common::load(jail).err();
        Ok(())
    });
    out.unwrap_or_else(|| panic!("config unexpectedly valid: {toml}"))
}

fn env_error(var: &str, value: &str) -> ConfigError {
    let mut out = None;
    Jail::expect_with(|jail| {
        jail.clear_env();
        jail.set_env(var, value);
        out = common::load(jail).err();
        Ok(())
    });
    out.unwrap_or_else(|| panic!("config unexpectedly valid: {var}={value}"))
}

fn assert_no_echo(err: &ConfigError, must_name: &str) {
    let msg = err.to_string();
    assert!(!msg.contains(PLANTED), "value echoed: {msg}");
    assert!(msg.contains(must_name), "missing {must_name:?}: {msg}");
}

#[test]
fn parent_table_given_a_string_does_not_echo() {
    for table in ["database", "security", "settings", "listen"] {
        let err = file_error(&format!("{table} = \"postgres://u:{PLANTED}@h/db\"\n"));
        assert_no_echo(&err, table);
        assert_no_echo(&err, "deskhaven.toml");
    }
}

#[test]
fn scalar_type_errors_do_not_echo() {
    assert_no_echo(
        &env_error("DESKHAVEN_SIGNAL__PORT", PLANTED),
        "DESKHAVEN_SIGNAL__PORT",
    );
    assert_no_echo(
        &env_error("DESKHAVEN_SIGNAL__ENABLED", PLANTED),
        "DESKHAVEN_SIGNAL__ENABLED",
    );
    let err = file_error(&format!("[listen]\nclient_api = \"{PLANTED}\"\n"));
    assert_no_echo(&err, "listen.client_api");
}

#[test]
fn enum_and_custom_type_errors_do_not_echo() {
    assert_no_echo(
        &file_error(&format!("[log]\nformat = \"{PLANTED}\"\n")),
        "log.format",
    );
    let proxies = format!("[security]\ntrusted_proxies = [\"{PLANTED}\"]\n");
    assert_no_echo(&file_error(&proxies), "security.trusted_proxies");
}

#[test]
fn log_filter_error_does_not_echo() {
    let err = file_error(&format!("[log]\nlevel = \"{PLANTED}=notalevel\"\n"));
    assert!(matches!(err, ConfigError::Invalid { .. }), "{err}");
    assert_no_echo(&err, "log.level");
}

#[test]
fn toml_syntax_error_reports_position_without_snippet() {
    let err = file_error(&format!("[database]\nurl = postgres://u:{PLANTED}@h/db\n"));
    assert!(matches!(err, ConfigError::TomlSyntax { .. }), "{err}");
    assert_no_echo(&err, "deskhaven.toml");
    assert!(err.to_string().contains("2:"), "{err}");
}

#[test]
fn empty_pin_table_is_error() {
    let err = file_error("[settings]\n\"auth.lockout\" = {}\n");
    assert!(err.to_string().contains("auth.lockout"), "{err}");
}

#[test]
fn env_names_differing_only_by_case_are_rejected() {
    let mut out = None;
    Jail::expect_with(|jail| {
        jail.clear_env();
        jail.set_env("DESKHAVEN_LOG__LEVEL", "info");
        jail.set_env("deskhaven_log__level", "debug");
        out = common::load(jail).err();
        Ok(())
    });
    let msg = out.expect("expected duplicate env error").to_string();
    assert!(
        msg.contains("DESKHAVEN_LOG__LEVEL") && msg.contains("deskhaven_log__level"),
        "{msg}"
    );
}
