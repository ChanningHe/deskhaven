//! End-to-end tests of `deskhaven key show` and the key-material config check.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../deskhaven-core/tests/fixtures/hbbs-libsodium"
);

fn fixture(name: &str) -> String {
    std::fs::read_to_string(Path::new(FIXTURE).join(name)).unwrap()
}

fn workdir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("cli_key")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_deskhaven"))
        .args(["--config", "none.toml"])
        .args(args)
        .env_clear()
        .current_dir(dir)
        .output()
        .unwrap()
}

/// `--config none.toml` must exist; an empty file means "all defaults".
fn prepare(name: &str) -> (PathBuf, PathBuf) {
    let dir = workdir(name);
    std::fs::write(dir.join("none.toml"), "").unwrap();
    let data = dir.join("data");
    (dir, data)
}

fn install_fixture(data: &Path) {
    std::fs::create_dir_all(data).unwrap();
    std::fs::write(data.join("id_ed25519"), fixture("id_ed25519")).unwrap();
    std::fs::write(data.join("id_ed25519.pub"), fixture("id_ed25519.pub")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(data.join("id_ed25519"), mode).unwrap();
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn key_show_prints_only_the_public_key() {
    let (dir, data) = prepare("show");
    install_fixture(&data);

    let out = run(&dir, &["--data-dir", data.to_str().unwrap(), "key", "show"]);

    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout),
        format!("{}\n", fixture("id_ed25519.pub"))
    );
    let all = format!("{}{}", text(&out.stdout), text(&out.stderr));
    assert!(!all.contains(&fixture("id_ed25519")[..20]), "{all}");
}

#[test]
fn key_show_without_key_fails_with_hint_and_creates_nothing() {
    let (dir, data) = prepare("show-missing");

    let out = run(&dir, &["--data-dir", data.to_str().unwrap(), "key", "show"]);

    assert!(!out.status.success());
    assert!(text(&out.stdout).is_empty());
    let stderr = text(&out.stderr);
    assert!(stderr.contains("id_ed25519"), "{stderr}");
    assert!(!data.exists());
}

#[test]
fn config_check_accepts_fresh_data_dir_and_valid_keys() {
    let (dir, data) = prepare("check-ok");
    let fresh = run(
        &dir,
        &["--data-dir", data.to_str().unwrap(), "config", "check"],
    );
    assert!(fresh.status.success(), "{}", text(&fresh.stderr));
    assert!(!data.exists(), "check must not generate keys");

    install_fixture(&data);
    let ok = run(
        &dir,
        &["--data-dir", data.to_str().unwrap(), "config", "check"],
    );
    assert!(ok.status.success(), "{}", text(&ok.stderr));
}

#[test]
fn config_check_rejects_mismatched_public_key() {
    let (dir, data) = prepare("check-mismatch");
    install_fixture(&data);
    std::fs::write(
        data.join("id_ed25519.pub"),
        "1UCLr9lEEMDh3d+QlbfUxenDpo1pdUDgL228tbJVRWA=",
    )
    .unwrap();

    let out = run(
        &dir,
        &["--data-dir", data.to_str().unwrap(), "config", "check"],
    );

    assert!(!out.status.success());
    let stderr = text(&out.stderr);
    assert!(stderr.contains("id_ed25519.pub"), "{stderr}");
}

#[test]
fn config_check_rejects_invalid_configured_master_key_without_echo() {
    let (dir, data) = prepare("check-master");
    let value = "dG9vLXNob3J0";
    let out = Command::new(env!("CARGO_BIN_EXE_deskhaven"))
        .args([
            "--config",
            "none.toml",
            "--data-dir",
            data.to_str().unwrap(),
        ])
        .args(["config", "check"])
        .env_clear()
        .env("DESKHAVEN_SECURITY__MASTER_KEY", value)
        .current_dir(&dir)
        .output()
        .unwrap();

    assert!(!out.status.success());
    let stderr = text(&out.stderr);
    assert!(stderr.contains("security.master_key"), "{stderr}");
    assert!(!stderr.contains(value), "{stderr}");
}
