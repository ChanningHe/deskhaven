//! deskhaven-contract may only be a dev-dependency (09-testing §2.2). Asked
//! of Cargo itself (`cargo metadata --no-deps`), so workspace inheritance,
//! renames (`name` is the package, `rename` the alias) and `[target.*]`
//! tables are already resolved: no workspace package may depend on it with a
//! kind other than `dev`. (Crates the server depends on could not anyway:
//! that would be a cycle.)
#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

const CONTRACT: &str = "deskhaven-contract";
const DEV_KIND: &str = "dev";

fn workspace_metadata() -> Value {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    let output = Command::new(cargo)
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--offline",
        ])
        .arg("--manifest-path")
        .arg(manifest)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn packages(metadata: &Value) -> &[Value] {
    metadata["packages"].as_array().unwrap()
}

/// `package: kind (target)` for every non-dev dependency on the contract
/// crate. `kind` is `null` for normal dependencies, `"build"`, or `"dev"`.
fn offenders(metadata: &Value) -> Vec<String> {
    packages(metadata)
        .iter()
        .flat_map(|package| {
            package["dependencies"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|dep| dep["name"] == CONTRACT && dep["kind"] != DEV_KIND)
                .map(move |dep| {
                    format!(
                        "{}: kind {} (target {}, rename {})",
                        package["name"], dep["kind"], dep["target"], dep["rename"]
                    )
                })
        })
        .collect()
}

#[test]
fn no_workspace_package_depends_on_contract_outside_dev_dependencies() {
    let metadata = workspace_metadata();
    let names: Vec<_> = packages(&metadata)
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&CONTRACT) && names.contains(&"deskhaven"),
        "workspace packages not found: {names:?}"
    );

    let found = offenders(&metadata);

    assert!(
        found.is_empty(),
        "deskhaven-contract must only be a dev-dependency: {found:?}"
    );
}

fn dep(kind: Value, target: Value, rename: Value) -> Value {
    json!({ "name": CONTRACT, "kind": kind, "target": target, "rename": rename })
}

#[test]
fn detector_catches_normal_renamed_build_and_target_dependencies() {
    let metadata = json!({ "packages": [
        { "name": "server", "dependencies": [
            dep(Value::Null, Value::Null, Value::Null),
            dep(Value::Null, Value::Null, json!("mirror")),
            dep(json!("build"), Value::Null, Value::Null),
            dep(Value::Null, json!("cfg(unix)"), Value::Null),
            dep(json!("dev"), Value::Null, Value::Null),
            dep(json!("dev"), json!("cfg(unix)"), json!("mirror")),
            { "name": "serde", "kind": null, "target": null, "rename": null }
        ] },
        { "name": "tests-only", "dependencies": [dep(json!("dev"), Value::Null, Value::Null)] }
    ] });

    let found = offenders(&metadata);

    assert_eq!(found.len(), 4, "{found:?}");
    assert!(
        found.iter().all(|f| f.starts_with("\"server\"")),
        "{found:?}"
    );
    assert!(found[1].contains("rename \"mirror\""), "{found:?}");
    assert!(found[2].contains("kind \"build\""), "{found:?}");
    assert!(found[3].contains("target \"cfg(unix)\""), "{found:?}");
}
