//! `xtask docs [--check]`: write the configuration reference ([`CONFIG_REFERENCE`]) from the
//! settings registry (`deskhaven_config::settings::schema`) and the bootstrap JSON Schema
//! (`deskhaven_config::bootstrap::json_schema`), or with `--check` fail when the committed
//! file differs from what the code generates (CI drift check).
//!
//! The prose around the tables is `configuration.template.md`; the tables are rendered by
//! [`render`]. A schema shape the renderer does not know is an error, so a new setting kind
//! cannot slip into the reference half-described.

mod render;

use std::path::Path;

use crate::error::XtaskError;
use crate::openapi::Mode;
use crate::sync;

/// The generated configuration reference, relative to the repository root.
pub const CONFIG_REFERENCE: &str = "docs/reference/configuration.md";

pub fn run(root: &Path, mode: Mode) -> Result<(), XtaskError> {
    let text = generate()?;
    sync::file(
        &root.join(CONFIG_REFERENCE),
        &text,
        mode,
        XtaskError::DocsDrift,
    )?;
    eprintln!("xtask: {CONFIG_REFERENCE} is up to date");
    Ok(())
}

fn generate() -> Result<String, XtaskError> {
    render::configuration(
        &deskhaven_config::bootstrap::json_schema(),
        &deskhaven_config::settings::schema(),
    )
    .map_err(XtaskError::Docs)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;

    use super::*;

    /// The table row of `key` (a line starting with its key cell).
    fn row<'a>(text: &'a str, key: &str) -> &'a str {
        let cell = format!("| `{key}` |");
        text.lines()
            .find(|line| line.starts_with(&cell))
            .unwrap_or_else(|| panic!("no row for {key}"))
    }

    #[test]
    fn a_setting_appears_with_its_range_and_default() {
        let text = generate().unwrap();

        let min_length = row(&text, "auth.password.min_length");
        assert!(min_length.contains("`8`–`256`"), "{min_length}");
        assert!(min_length.contains("| `10` |"), "{min_length}");
        let retention = row(&text, "audit.retention.login");
        assert!(retention.contains("`1d`–`3650d`, or `0`"), "{retention}");
        assert!(retention.contains("| `180d` |"), "{retention}");
        let tfa = row(&text, "auth.tfa.enforce");
        assert!(tfa.contains("`off`, `admins`, `all`"), "{tfa}");
        assert!(row(&text, "smtp.password").contains("| unset |"));
    }

    #[test]
    fn a_bootstrap_key_appears_with_its_variable_and_default() {
        let text = generate().unwrap();

        let client_api = row(&text, "listen.client_api");
        assert!(
            client_api.contains("`DESKHAVEN_LISTEN__CLIENT_API`"),
            "{client_api}"
        );
        assert!(client_api.contains("`0.0.0.0:21114`"), "{client_api}");
        let master_key = row(&text, "security.master_key");
        assert!(master_key.contains("| secret |"), "{master_key}");
        assert!(row(&text, "log.format").contains("`pretty`, `json`"));
        assert!(row(&text, "security.trusted_proxies").contains("list of cidr"));
    }

    #[test]
    fn every_setting_and_rule_is_listed() {
        let text = generate().unwrap();

        for setting in deskhaven_config::settings::all() {
            row(&text, setting.key());
        }
        assert!(text.contains("auth.session_idle_ttl must not exceed auth.session_ttl"));
        for slot in ["@BOOTSTRAP@", "@SETTINGS@", "@RULES@"] {
            assert!(!text.contains(slot), "{slot} left unfilled");
        }
    }

    #[test]
    fn an_unknown_setting_type_is_an_error() {
        let settings = json!({
            "settings": [{"key": "a.b", "category": "a", "type": "colour", "default": "red",
                          "description": "x", "sensitive": false}],
            "rules": [],
        });

        let result = render::configuration(&deskhaven_config::bootstrap::json_schema(), &settings);

        assert!(result.unwrap_err().contains("colour"));
    }

    #[test]
    fn check_fails_on_drift_and_write_fixes_it() {
        let dir = tempfile::tempdir().unwrap();
        let reference = dir.path().join(CONFIG_REFERENCE);
        fs::create_dir_all(reference.parent().unwrap()).unwrap();

        assert!(matches!(
            run(dir.path(), Mode::Check),
            Err(XtaskError::DocsDrift(_))
        ));
        run(dir.path(), Mode::Write).unwrap();
        run(dir.path(), Mode::Check).unwrap();
        fs::write(&reference, "stale\n").unwrap();
        assert!(matches!(
            run(dir.path(), Mode::Check),
            Err(XtaskError::DocsDrift(_))
        ));
    }

    #[test]
    fn the_committed_reference_is_what_the_code_generates() {
        run(crate::repo_root(), Mode::Check).unwrap();
    }
}
