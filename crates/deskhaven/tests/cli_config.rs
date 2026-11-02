//! End-to-end tests of `deskhaven config ...` and `serve` config handling.
//! Each run gets a cleared environment and its own directory.

// Test code: panicking helpers are fine; figment::Jail fixes the closure error type.
#![allow(clippy::unwrap_used, clippy::panic, clippy::result_large_err)]

use std::path::PathBuf;
use std::process::{Command, Output};

fn workdir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("cli_config")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &PathBuf, args: &[&str], env: &[(&str, &str)]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_deskhaven"))
        .args(args)
        .env_clear()
        .envs(env.iter().copied())
        .current_dir(dir)
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn line_for<'a>(stdout: &'a str, key: &str) -> &'a str {
    let prefix = format!("{key} ");
    stdout
        .lines()
        .find(|l| l.starts_with(&prefix))
        .unwrap_or_else(|| panic!("{key}: {stdout}"))
}

#[test]
fn print_effective_redacts_secrets_and_labels_sources() {
    let dir = workdir("print");
    let file = dir.join("deskhaven.toml");
    std::fs::write(&file, "[database]\nurl = \"postgres://u:pw@db/dh\"\n").unwrap();
    let file_arg = file.to_str().unwrap();

    let out = run(
        &dir,
        &[
            "--config",
            file_arg,
            "--data-dir",
            "/cli/data",
            "config",
            "print",
            "--effective",
        ],
        &[("DESKHAVEN_SECURITY__MASTER_KEY", "super-secret")],
    );

    let stdout = text(&out.stdout);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(
        !stdout.contains("super-secret") && !stdout.contains("pw@"),
        "{stdout}"
    );
    let mk = line_for(&stdout, "security.master_key");
    assert!(mk.contains("\"<redacted>\"") && mk.ends_with("# env:DESKHAVEN_SECURITY__MASTER_KEY"));
    let db = line_for(&stdout, "database.url");
    assert!(
        db.contains("\"<redacted>\"") && db.ends_with(&format!("# file:{file_arg}")),
        "{db}"
    );
    assert!(line_for(&stdout, "signal.port").ends_with("# default"));
    assert!(line_for(&stdout, "data_dir").ends_with("# cli"));
}

#[test]
fn check_fails_on_unknown_file_key() {
    let dir = workdir("unknown_key");
    std::fs::write(dir.join("deskhaven.toml"), "[listen]\ntypo = 1\n").unwrap();

    let out = run(
        &dir,
        &["--config", "deskhaven.toml", "config", "check"],
        &[],
    );

    assert!(!out.status.success());
    let stderr = text(&out.stderr);
    assert!(
        stderr.contains("typo") && stderr.contains("deskhaven.toml"),
        "{stderr}"
    );
}

#[test]
fn check_warns_on_unknown_env_but_succeeds() {
    let dir = workdir("unknown_env");
    std::fs::write(dir.join("deskhaven.toml"), "").unwrap();

    let out = run(
        &dir,
        &["config", "check"],
        &[
            ("DESKHAVEN_CONFIG", "deskhaven.toml"),
            ("DESKHAVEN_TYPO", "1"),
        ],
    );

    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(text(&out.stderr).contains("DESKHAVEN_TYPO"));
    assert!(text(&out.stdout).contains("configuration OK"));
}

#[test]
fn schema_is_json_and_forbids_unknown_keys() {
    let dir = workdir("schema");

    let out = run(&dir, &["config", "schema"], &[]);

    assert!(out.status.success(), "{}", text(&out.stderr));
    let schema: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(schema["additionalProperties"], serde_json::json!(false));
}

#[test]
fn serve_stub_loads_config_and_exits_zero() {
    let dir = workdir("serve");
    std::fs::write(dir.join("deskhaven.toml"), "").unwrap();

    let ok = run(&dir, &["--config", "deskhaven.toml", "serve"], &[]);
    let bad = run(&dir, &["--config", "missing.toml", "serve"], &[]);

    assert!(ok.status.success(), "{}", text(&ok.stderr));
    assert!(!bad.status.success());
    assert!(text(&bad.stderr).contains("missing.toml"));
}
