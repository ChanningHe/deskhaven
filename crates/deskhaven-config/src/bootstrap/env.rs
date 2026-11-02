//! Classification of `DESKHAVEN_*` variables and `<KEY>_FILE` indirection.
//!
//! Env values never go through figment's own `Env` provider: they are typed
//! by [`KnownKeys::typed_value`], the single parser shared with `_FILE` and CLI.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

use figment::value::{Dict, Value};

use super::error::{ConfigError, Warning};
use super::keys::{KnownKeys, dict_from};

pub(crate) const ENV_PREFIX: &str = "DESKHAVEN_";
pub(crate) const KEY_SEPARATOR: &str = "__";
pub(crate) const CONFIG_VAR: &str = "DESKHAVEN_CONFIG";
const FILE_SUFFIX: &str = "_file";

/// How every `DESKHAVEN_*` variable is interpreted, keyed by dotted path.
#[derive(Default)]
pub(crate) struct EnvPlan {
    /// path -> (variable, value).
    pub plain: BTreeMap<String, (String, String)>,
    /// path -> (variable, file to read the value from).
    pub indirect: BTreeMap<String, (String, PathBuf)>,
    /// `$DESKHAVEN_CONFIG`, if set and non-empty.
    pub config_path: Option<PathBuf>,
    /// The variable name that carried the config path (even if empty).
    config_var: Option<String>,
    pub warnings: Vec<Warning>,
}

/// Variable names and file paths only; plain values may be secrets.
impl fmt::Debug for EnvPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let plain: BTreeMap<&str, &str> = self
            .plain
            .iter()
            .map(|(path, (var, _))| (path.as_str(), var.as_str()))
            .collect();
        f.debug_struct("EnvPlan")
            .field("plain", &plain)
            .field("indirect", &self.indirect)
            .field("config_path", &self.config_path)
            .field("warnings", &self.warnings)
            .finish()
    }
}

impl EnvPlan {
    /// The variable that supplied `path`, for provenance.
    pub(crate) fn var_for(&self, path: &str) -> Option<&str> {
        let plain = self.plain.get(path).map(|(var, _)| var);
        plain
            .or_else(|| self.indirect.get(path).map(|(var, _)| var))
            .map(String::as_str)
    }
}

/// The env layers: (plain values, `_FILE` values).
pub(crate) struct EnvLayers {
    pub plain: Dict,
    pub indirect: Dict,
}

pub(crate) fn plan(
    vars: impl IntoIterator<Item = (OsString, OsString)>,
    known: &KnownKeys,
) -> Result<EnvPlan, ConfigError> {
    let mut plan = EnvPlan::default();
    for (name, value) in vars {
        let var = name.to_string_lossy().trim().to_owned();
        let Some(rest) = strip_prefix_ignore_case(&var, ENV_PREFIX).map(str::to_owned) else {
            continue;
        };
        let (Some(_), Some(value)) = (name.to_str(), value.to_str()) else {
            plan.warnings.push(Warning::NonUnicodeEnv { var });
            continue;
        };
        classify(&mut plan, known, var, &rest, value)?;
    }
    reject_conflicts(&plan)?;
    Ok(plan)
}

/// Schema leaves win (`SIGNAL__KEY_FILE` is the plain `signal.key_file`);
/// then a `_FILE` suffix over a known key (schema or pin) is indirection;
/// then any other pin path is a plain pin. Names that differ only by case
/// map to the same key and are rejected.
fn classify(
    plan: &mut EnvPlan,
    known: &KnownKeys,
    var: String,
    rest: &str,
    value: &str,
) -> Result<(), ConfigError> {
    if var.eq_ignore_ascii_case(CONFIG_VAR) {
        if let Some(other) = &plan.config_var {
            return Err(duplicate(other, &var));
        }
        plan.config_path = (!value.trim().is_empty()).then(|| PathBuf::from(value));
        plan.config_var = Some(var);
        return Ok(());
    }
    let path = to_path(rest);
    let base = path
        .strip_suffix(FILE_SUFFIX)
        .filter(|base| known.contains(base));
    match base {
        _ if known.is_schema_leaf(&path) => {
            insert_unique(&mut plan.plain, path, var, value.to_owned())
        }
        Some(base) => insert_unique(
            &mut plan.indirect,
            base.to_owned(),
            var,
            PathBuf::from(value),
        ),
        None if known.contains(&path) => {
            insert_unique(&mut plan.plain, path, var, value.to_owned())
        }
        None => {
            plan.warnings.push(Warning::UnknownEnv { var });
            Ok(())
        }
    }
}

fn insert_unique<V>(
    map: &mut BTreeMap<String, (String, V)>,
    path: String,
    var: String,
    value: V,
) -> Result<(), ConfigError> {
    if let Some((other, _)) = map.get(&path) {
        return Err(duplicate(other, &var));
    }
    map.insert(path, (var, value));
    Ok(())
}

/// Deterministic regardless of environment iteration order.
fn duplicate(a: &str, b: &str) -> ConfigError {
    let (first, second) = if a <= b { (a, b) } else { (b, a) };
    ConfigError::EnvDuplicate {
        first: first.to_owned(),
        second: second.to_owned(),
    }
}

fn reject_conflicts(plan: &EnvPlan) -> Result<(), ConfigError> {
    for (path, (file_var, _)) in &plan.indirect {
        if let Some((var, _)) = plan.plain.get(path) {
            return Err(ConfigError::EnvConflict {
                var: var.clone(),
                file_var: file_var.clone(),
            });
        }
    }
    Ok(())
}

/// Types plain values and reads `_FILE` targets. Both layers are also checked
/// together so a pin cannot be shadowed across them (`A` vs `A__B_FILE`).
pub(crate) fn layers(plan: &EnvPlan, known: &KnownKeys) -> Result<EnvLayers, ConfigError> {
    let plain: Vec<(String, Value)> = plan
        .plain
        .iter()
        .map(|(path, (_, raw))| (path.clone(), known.typed_value(path, raw)))
        .collect();
    let indirect = read_indirect(plan, known)?;
    dict_from(plain.iter().chain(&indirect).cloned().collect())?;
    Ok(EnvLayers {
        plain: dict_from(plain)?,
        indirect: dict_from(indirect)?,
    })
}

/// One trailing newline is stripped, as secret files usually end with one.
fn read_indirect(plan: &EnvPlan, known: &KnownKeys) -> Result<Vec<(String, Value)>, ConfigError> {
    let mut values = Vec::with_capacity(plan.indirect.len());
    for (path, (var, file)) in &plan.indirect {
        let content = std::fs::read_to_string(file).map_err(|source| ConfigError::EnvFileRead {
            var: var.clone(),
            path: file.clone(),
            source,
        })?;
        let trimmed = content.strip_suffix('\n').unwrap_or(&content);
        let trimmed = trimmed.strip_suffix('\r').unwrap_or(trimmed);
        values.push((path.clone(), known.typed_value(path, trimmed)));
    }
    Ok(values)
}

fn strip_prefix_ignore_case<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let (head, tail) = (s.get(..prefix.len())?, s.get(prefix.len()..)?);
    head.eq_ignore_ascii_case(prefix).then_some(tail)
}

fn to_path(rest: &str) -> String {
    rest.to_ascii_lowercase().replace(KEY_SEPARATOR, ".")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        pairs
            .iter()
            .map(|(k, v)| (OsString::from(k), OsString::from(v)))
            .collect()
    }

    #[test]
    fn classifies_plain_indirect_config_and_unknown() {
        let known = KnownKeys::new().unwrap();
        let plan = plan(
            vars(&[
                ("DESKHAVEN_LISTEN__ADMIN", "127.0.0.1:1"),
                ("deskhaven_log__level", "debug"),
                ("DESKHAVEN_DATABASE__URL_FILE", "/run/secrets/db"),
                ("DESKHAVEN_CONFIG", "/etc/x.toml"),
                ("DESKHAVEN_NOPE", "1"),
                ("PATH", "/bin"),
            ]),
            &known,
        )
        .unwrap();

        assert_eq!(
            plan.var_for("listen.admin"),
            Some("DESKHAVEN_LISTEN__ADMIN")
        );
        assert_eq!(plan.var_for("log.level"), Some("deskhaven_log__level"));
        assert_eq!(
            plan.var_for("database.url"),
            Some("DESKHAVEN_DATABASE__URL_FILE")
        );
        assert_eq!(plan.config_path, Some(PathBuf::from("/etc/x.toml")));
        assert_eq!(
            plan.warnings,
            vec![Warning::UnknownEnv {
                var: "DESKHAVEN_NOPE".into()
            }]
        );
    }

    #[test]
    fn debug_shows_variable_names_not_values() {
        let known = KnownKeys::new().unwrap();
        let plan = plan(
            vars(&[("DESKHAVEN_SECURITY__MASTER_KEY", "HUNTER2")]),
            &known,
        )
        .unwrap();
        let debug = format!("{plan:?}");
        assert!(debug.contains("DESKHAVEN_SECURITY__MASTER_KEY") && !debug.contains("HUNTER2"));
    }

    #[test]
    fn empty_segment_is_unknown() {
        let known = KnownKeys::new().unwrap();
        let plan = plan(vars(&[("DESKHAVEN_LISTEN____ADMIN", "x")]), &known).unwrap();
        assert!(plan.plain.is_empty());
        assert_eq!(plan.warnings.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_value_warns() {
        use std::os::unix::ffi::OsStringExt;
        let known = KnownKeys::new().unwrap();
        let bad = OsString::from_vec(vec![0xff, 0xfe]);
        let plan = plan([(OsString::from("DESKHAVEN_LOG__LEVEL"), bad)], &known).unwrap();
        assert_eq!(
            plan.warnings,
            vec![Warning::NonUnicodeEnv {
                var: "DESKHAVEN_LOG__LEVEL".into()
            }]
        );
    }
}
