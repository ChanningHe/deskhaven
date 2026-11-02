//! Reading the TOML config file. Parsed here rather than by figment so that
//! syntax errors report a position only: toml's rendered errors quote the
//! offending source line, which may hold credentials.

use std::path::{Path, PathBuf};

use figment::value::Dict;

use super::error::ConfigError;
use super::pins::normalize_root;

/// Reads `path` and returns its table with `[settings]` pins normalized.
pub(crate) fn read(path: &Path) -> Result<Dict, ConfigError> {
    let content = std::fs::read_to_string(path).map_err(|source| ConfigError::ConfigFileRead {
        path: path.to_owned(),
        source,
    })?;
    let table: Dict = toml::from_str(&content).map_err(|e| syntax_error(path, &content, &e))?;
    normalize_root(table).map_err(|problem| ConfigError::Invalid {
        problems: vec![format!("{problem} (in {})", path.display())],
    })
}

/// Position plus toml's short message; the source excerpt is never included.
fn syntax_error(path: &Path, content: &str, err: &toml::de::Error) -> ConfigError {
    let (line, column) = err
        .span()
        .map_or((0, 0), |span| line_col(content, span.start));
    let reason = err
        .message()
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("; ");
    ConfigError::TomlSyntax {
        path: PathBuf::from(path),
        line,
        column,
        reason,
    }
}

/// 1-based line and column (in characters) of byte `offset`.
fn line_col(content: &str, offset: usize) -> (usize, usize) {
    let before = content.get(..offset).unwrap_or(content);
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_is_one_based() {
        assert_eq!(line_col("a = 1\nbc = x", 11), (2, 6));
        assert_eq!(line_col("x", 0), (1, 1));
    }
}
