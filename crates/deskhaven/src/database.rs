//! Database connection settings derived from the bootstrap `[database]` table.

use anyhow::Context;
use deskhaven_config::bootstrap::Bootstrap;
use deskhaven_db::DbOptions;
use sea_orm::DatabaseConnection;

/// SQLite file inside data_dir when `database.url` is empty (05-config §2.2).
pub const DEFAULT_SQLITE_FILE: &str = "deskhaven.db";

/// `database.url`, or `sqlite://{data_dir}/deskhaven.db` when empty.
/// May embed credentials: never log it (use [`backend`]).
pub fn url(config: &Bootstrap) -> anyhow::Result<String> {
    let configured = config.database.url.expose();
    if !configured.is_empty() {
        return Ok(configured.to_owned());
    }
    let path = config.data_dir.join(DEFAULT_SQLITE_FILE);
    let path = path
        .to_str()
        .context("data_dir must be valid UTF-8 to derive the SQLite database URL")?;
    Ok(format!("sqlite://{}", escape_path(path)))
}

/// The URL parser percent-decodes the path and cuts it at `?`.
fn escape_path(path: &str) -> String {
    path.replace('%', "%25")
        .replace('?', "%3F")
        .replace('#', "%23")
}

/// Pool settings for `url` (read-write).
pub fn options(config: &Bootstrap, url: String) -> DbOptions {
    let mut options = DbOptions::new(url);
    options.max_connections = config.database.max_connections;
    options
}

/// [`deskhaven_db::connect`] with an operator hint (sqlx reports a refused or
/// unroutable server only as a pool timeout).
pub async fn connect(options: &DbOptions) -> anyhow::Result<DatabaseConnection> {
    let backend = backend(&options.url);
    deskhaven_db::connect(options).await.with_context(|| {
        format!("cannot open the {backend} database (check database.url and that the server is reachable)")
    })
}

/// Backend name, the only part of a database URL that may be logged.
pub fn backend(url: &str) -> &'static str {
    if url.starts_with("sqlite:") {
        "sqlite"
    } else {
        "postgres"
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use deskhaven_config::Secret;

    use super::*;

    fn config(data_dir: &str, url: &str) -> Bootstrap {
        let mut config = Bootstrap {
            data_dir: PathBuf::from(data_dir),
            ..Bootstrap::default()
        };
        config.database.url = Secret::new(url);
        config
    }

    #[test]
    fn empty_url_means_sqlite_in_data_dir() {
        let url = url(&config("/var/lib/dh", "")).unwrap();

        assert_eq!(url, "sqlite:///var/lib/dh/deskhaven.db");
        assert_eq!(
            deskhaven_db::sqlite_file(&url).as_deref(),
            Some(Path::new("/var/lib/dh/deskhaven.db"))
        );
    }

    #[test]
    fn special_characters_in_data_dir_round_trip() {
        let url = url(&config("/srv/a%b?c#d", "")).unwrap();

        assert_eq!(
            deskhaven_db::sqlite_file(&url).as_deref(),
            Some(Path::new("/srv/a%b?c#d/deskhaven.db"))
        );
    }

    #[test]
    fn configured_url_wins() {
        let url = url(&config("/d", "postgres://u:p@h/db")).unwrap();

        assert_eq!(url, "postgres://u:p@h/db");
        assert_eq!(backend(&url), "postgres");
        assert_eq!(backend("sqlite::memory:"), "sqlite");
    }

    #[test]
    fn options_take_the_pool_size() {
        let mut cfg = config("/d", "");
        cfg.database.max_connections = 3;

        assert_eq!(options(&cfg, "sqlite::memory:".into()).max_connections, 3);
    }
}
