//! End-to-end tests of `deskhaven config ...` and `serve` config handling.
//! Each run gets a cleared environment and its own directory.

// Test code: panicking helpers are fine; figment::Jail fixes the closure error type.
#![allow(clippy::unwrap_used, clippy::panic, clippy::result_large_err)]

use std::net::TcpListener;
use std::path::{Path, PathBuf};
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

/// Own data_dir and an OS-assigned port: `config check` probes both.
fn isolated_config(dir: &Path, extra: &str) -> String {
    format!(
        "data_dir = \"{}\"\n{extra}\n[listen]\nclient_api = \"127.0.0.1:0\"\n",
        dir.join("data").display()
    )
}

#[test]
fn check_warns_on_unknown_env_but_succeeds() {
    let dir = workdir("unknown_env");
    std::fs::write(dir.join("deskhaven.toml"), isolated_config(&dir, "")).unwrap();

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
fn print_without_effective_lists_only_explicit_keys() {
    let dir = workdir("print_explicit");
    std::fs::write(dir.join("deskhaven.toml"), "[log]\nformat = \"json\"\n").unwrap();

    let out = run(
        &dir,
        &["--config", "deskhaven.toml", "config", "print"],
        &[],
    );

    let stdout = text(&out.stdout);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(
        line_for(&stdout, "log.format").contains("# file:"),
        "{stdout}"
    );
    assert!(!stdout.contains("# default"), "{stdout}");
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
fn serve_fails_on_missing_config_file() {
    let dir = workdir("serve");

    let bad = run(&dir, &["--config", "missing.toml", "serve"], &[]);

    assert!(!bad.status.success());
    assert!(text(&bad.stderr).contains("missing.toml"));
}

#[test]
fn check_fails_when_a_listen_port_is_in_use() {
    let dir = workdir("port_in_use");
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = taken.local_addr().unwrap();
    let config = format!(
        "data_dir = \"{}\"\n[listen]\nclient_api = \"{addr}\"\n",
        dir.join("data").display()
    );
    std::fs::write(dir.join("deskhaven.toml"), config).unwrap();

    let out = run(
        &dir,
        &["--config", "deskhaven.toml", "config", "check"],
        &[],
    );

    assert!(!out.status.success());
    let stderr = text(&out.stderr);
    assert!(
        stderr.contains("listen.client_api") && stderr.contains(&addr.to_string()),
        "{stderr}"
    );
}

#[test]
fn check_fails_on_unreachable_database_without_echoing_secrets() {
    let dir = workdir("db_down");
    std::fs::write(dir.join("deskhaven.toml"), isolated_config(&dir, "")).unwrap();
    let password = "pw-Hunter2-secret";
    let url = format!("postgres://deskhaven:{password}@127.0.0.1:1/deskhaven");

    let out = run(
        &dir,
        &["--config", "deskhaven.toml", "config", "check"],
        &[("DESKHAVEN_DATABASE__URL", &url)],
    );

    assert!(!out.status.success());
    let all = format!("{}{}", text(&out.stdout), text(&out.stderr));
    assert!(all.contains("database"), "{all}");
    assert!(
        !all.contains(password) && !all.contains("postgres://"),
        "{all}"
    );
}

#[test]
fn check_rejects_invalid_pin_without_echoing_the_value() {
    let dir = workdir("bad_pin");
    std::fs::write(dir.join("deskhaven.toml"), isolated_config(&dir, "")).unwrap();

    let out = run(
        &dir,
        &["--config", "deskhaven.toml", "config", "check"],
        &[(
            "DESKHAVEN_SETTINGS__AUTH__SESSION_TTL",
            "forty-two-SECRETISH",
        )],
    );

    assert!(!out.status.success());
    let stderr = text(&out.stderr);
    assert!(stderr.contains("auth.session_ttl"), "{stderr}");
    assert!(!stderr.contains("SECRETISH"), "{stderr}");
}

#[test]
fn check_inspects_the_database_read_only() {
    let dir = workdir("db_read_only");
    std::fs::write(dir.join("deskhaven.toml"), isolated_config(&dir, "")).unwrap();
    let db = dir.join("data").join("deskhaven.db");

    let fresh = run(
        &dir,
        &["--config", "deskhaven.toml", "config", "check"],
        &[],
    );

    assert!(fresh.status.success(), "{}", text(&fresh.stderr));
    assert!(text(&fresh.stderr).contains("created on first start"));
    assert!(!dir.join("data").exists(), "check must not create data_dir");

    // A zero-byte file is an empty SQLite database: every migration pending.
    std::fs::create_dir_all(dir.join("data")).unwrap();
    std::fs::write(&db, b"").unwrap();
    let empty = run(
        &dir,
        &["--config", "deskhaven.toml", "config", "check"],
        &[],
    );

    assert!(empty.status.success(), "{}", text(&empty.stderr));
    assert!(
        text(&empty.stderr).contains("pending migration"),
        "{}",
        text(&empty.stderr)
    );
    assert_eq!(
        std::fs::metadata(&db).unwrap().len(),
        0,
        "check wrote to the database"
    );
}
