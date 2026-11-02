//! deskhaven-contract may only be a dev-dependency (09-testing §2.2): no
//! workspace member may list it under `[dependencies]` or
//! `[build-dependencies]` (plain or `[target.*]`), under any rename.
//! (Crates the server depends on could not anyway: that would be a cycle.)
#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};

use toml::Value;

const CONTRACT: &str = "deskhaven-contract";
const FORBIDDEN_TABLES: [&str; 2] = ["dependencies", "build-dependencies"];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read_toml(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap();
    toml::from_str(&text).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

/// Member manifests from `[workspace] members` (`dir` or `dir/*`).
fn member_manifests(root: &Path) -> Vec<PathBuf> {
    let workspace = read_toml(&root.join("Cargo.toml"));
    let members = workspace["workspace"]["members"].as_array().unwrap();
    members
        .iter()
        .map(|m| m.as_str().unwrap())
        .flat_map(|member| match member.strip_suffix("/*") {
            Some(dir) => std::fs::read_dir(root.join(dir))
                .unwrap()
                .map(|e| e.unwrap().path().join("Cargo.toml"))
                .filter(|p| p.is_file())
                .collect(),
            None => vec![root.join(member).join("Cargo.toml")],
        })
        .collect()
}

/// Dependency tables that ship with the crate.
fn production_tables(manifest: &Value) -> Vec<&toml::Table> {
    let top = FORBIDDEN_TABLES.iter().filter_map(|t| manifest.get(*t));
    let targets = manifest
        .get("target")
        .and_then(Value::as_table)
        .into_iter()
        .flat_map(|targets| targets.values())
        .flat_map(|target| FORBIDDEN_TABLES.iter().filter_map(|t| target.get(*t)));
    top.chain(targets).filter_map(Value::as_table).collect()
}

fn names_contract(key: &str, spec: &Value) -> bool {
    key == CONTRACT || spec.get("package").and_then(Value::as_str) == Some(CONTRACT)
}

fn offenders(manifest_path: &Path) -> Vec<String> {
    let manifest = read_toml(manifest_path);
    production_tables(&manifest)
        .into_iter()
        .flat_map(|table| table.iter())
        .filter(|(key, spec)| names_contract(key, spec))
        .map(|(key, _)| format!("{}: {key}", manifest_path.display()))
        .collect()
}

#[test]
fn no_member_depends_on_contract_outside_dev_dependencies() {
    let manifests = member_manifests(&workspace_root());
    assert!(
        manifests.len() > 5,
        "workspace members not found: {manifests:?}"
    );

    let found: Vec<_> = manifests.iter().flat_map(|m| offenders(m)).collect();

    assert!(
        found.is_empty(),
        "deskhaven-contract must only be a dev-dependency: {found:?}"
    );
}

#[test]
fn detector_catches_normal_renamed_and_target_dependencies() {
    let manifest: Value = toml::from_str(
        r#"
        [dependencies]
        mirror = { package = "deskhaven-contract", path = "x" }
        [target.'cfg(unix)'.build-dependencies]
        deskhaven-contract = { workspace = true }
        [dev-dependencies]
        deskhaven-contract = { workspace = true }
        "#,
    )
    .unwrap();

    let hits: Vec<_> = production_tables(&manifest)
        .into_iter()
        .flat_map(|t| t.iter())
        .filter(|(k, v)| names_contract(k, v))
        .map(|(k, _)| k.clone())
        .collect();

    assert_eq!(hits, ["mirror", "deskhaven-contract"]);
}
