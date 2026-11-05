//! End-to-end tests of `deskhaven healthcheck`: it resolves the listen address
//! from the same configuration as `serve` and reports `/readyz` in its exit code.
#![cfg(unix)]
// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]

mod support;

use std::path::Path;
use std::process::{Command, Output};

use support::*;

/// `config` with the client API on a fixed loopback port: the healthcheck reads
/// the port from the configuration, so `:0` would not do.
fn config_on(dir: &Path, port: u16) -> String {
    config(dir, "").replace("127.0.0.1:0", &format!("127.0.0.1:{port}"))
}

fn healthcheck(dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_deskhaven"))
        .args(["--config", "deskhaven.toml", "healthcheck"])
        .env_clear()
        .current_dir(dir)
        .output()
        .unwrap()
}

#[test]
fn succeeds_against_a_ready_server() {
    let dir = workdir("healthcheck_ready");
    let mut server = start(&dir, &config_on(&dir, free_port()), &[]);
    wait_ready(server.listening("client_api"));

    let output = healthcheck(&dir);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let (status, _) = server.terminate();
    assert_eq!(status.code(), Some(0), "{}", server.logs());
}

#[test]
fn fails_when_nothing_listens() {
    let dir = workdir("healthcheck_down");
    std::fs::write(dir.join("deskhaven.toml"), config_on(&dir, free_port())).unwrap();

    let output = healthcheck(&dir);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("cannot connect"), "{stderr}");
}

#[test]
fn fails_on_an_invalid_configuration() {
    let dir = workdir("healthcheck_bad_config");
    std::fs::write(dir.join("deskhaven.toml"), "no_such_key = 1\n").unwrap();

    let output = healthcheck(&dir);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
}
