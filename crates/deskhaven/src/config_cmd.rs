//! `deskhaven config ...` subcommands.

use std::io::Write;

use anyhow::Context;
use deskhaven_config::Source;
use deskhaven_config::bootstrap::{EffectiveEntry, LoadOptions, Loaded, json_schema, load};

/// A readiness check run by `config check` after the configuration loaded.
pub struct Check {
    pub name: &'static str,
    pub run: fn(&Loaded) -> anyhow::Result<()>,
}

/// Checks beyond bootstrap validation (which `load` already performs), all
/// read-only; cheap ones first.
const CHECKS: &[Check] = &[
    Check {
        name: "settings pins",
        run: crate::check::settings_pins,
    },
    Check {
        name: "key material",
        run: crate::key_cmd::check,
    },
    Check {
        name: "listen ports",
        run: crate::check::listen_ports,
    },
    Check {
        name: "database",
        run: crate::check::database,
    },
];

/// Loads the configuration and reports warnings on stderr.
pub fn load_reporting(options: &LoadOptions) -> anyhow::Result<Loaded> {
    let loaded = load(options).context("failed to load configuration")?;
    let mut err = std::io::stderr().lock();
    for warning in &loaded.warnings {
        writeln!(err, "warning: {warning}")?;
    }
    Ok(loaded)
}

pub fn check(options: &LoadOptions) -> anyhow::Result<()> {
    let loaded = load_reporting(options)?;
    for check in CHECKS {
        (check.run)(&loaded).with_context(|| format!("check `{}` failed", check.name))?;
    }
    let file = loaded.config_file.as_ref().map_or_else(
        || "no file, defaults + env".to_owned(),
        |p| p.display().to_string(),
    );
    writeln!(std::io::stdout().lock(), "configuration OK ({file})")?;
    Ok(())
}

/// Prints `key = value  # source`; with `include_defaults` every key, else
/// only keys some layer set explicitly.
pub fn print(options: &LoadOptions, include_defaults: bool) -> anyhow::Result<()> {
    let loaded = load_reporting(options)?;
    let rows: Vec<(String, String)> = loaded
        .effective()
        .iter()
        .filter(|e| include_defaults || e.source != Source::Default)
        .map(render)
        .collect();
    let width = rows.iter().map(|(left, _)| left.len()).max().unwrap_or(0);
    let mut out = std::io::stdout().lock();
    for (left, source) in rows {
        writeln!(out, "{left:<width$}  # {source}")?;
    }
    Ok(())
}

pub fn schema() -> anyhow::Result<()> {
    let text = serde_json::to_string_pretty(&json_schema())?;
    writeln!(std::io::stdout().lock(), "{text}")?;
    Ok(())
}

fn render(entry: &EffectiveEntry) -> (String, String) {
    (
        format!("{} = {}", entry.key, entry.value),
        entry.source.to_string(),
    )
}
