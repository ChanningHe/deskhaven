//! `deskhaven config ...` subcommands.

use std::io::Write;

use anyhow::{Context, bail};
use deskhaven_config::Source;
use deskhaven_config::bootstrap::{EffectiveEntry, LoadOptions, Loaded, json_schema, load};

use crate::log_safe;

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

/// Runs every check, reporting each failure on stderr as it happens; fails
/// if any did. Read-only throughout.
pub fn check(options: &LoadOptions) -> anyhow::Result<()> {
    let loaded = load_reporting(options)?;
    let failed = CHECKS
        .iter()
        .filter(|check| !run_reporting(check, &loaded))
        .count();
    if failed > 0 {
        bail!("{failed} of {} checks failed", CHECKS.len());
    }
    let file = loaded.config_file.as_ref().map_or_else(
        || "no file, defaults + env".to_owned(),
        |p| p.display().to_string(),
    );
    writeln!(std::io::stdout().lock(), "configuration OK ({file})")?;
    Ok(())
}

/// Whether `check` passed; a failure is printed with its secret-safe chain.
fn run_reporting(check: &Check, loaded: &Loaded) -> bool {
    let Err(err) = (check.run)(loaded) else {
        return true;
    };
    // Nothing sensible remains if stderr itself is gone; the count still fails the run.
    let _ = writeln!(
        std::io::stderr().lock(),
        "error: check `{}` failed: {}",
        check.name,
        log_safe::render(&err)
    );
    false
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
