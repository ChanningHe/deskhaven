//! `config check` probes beyond bootstrap validation. All read-only: nothing
//! is created, migrated or generated, and sockets are released at once.

use std::io::Write;
use std::net::TcpListener;
use std::time::Duration;

use anyhow::{Context, bail};
use deskhaven_config::bootstrap::{Bootstrap, Loaded};
use deskhaven_config::settings::PinSet;

use crate::database;

/// `config check` gives up on the database sooner than startup does.
const CHECK_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Every `[settings]` pin names a known setting and coerces to its type.
pub fn settings_pins(loaded: &Loaded) -> anyhow::Result<()> {
    PinSet::from_bootstrap(&loaded.config.settings)?;
    Ok(())
}

/// The HTTP listen addresses can be bound now (another process may hold
/// them, e.g. a running server). The signal ports join with M1-06.
pub fn listen_ports(loaded: &Loaded) -> anyhow::Result<()> {
    let listen = &loaded.config.listen;
    let admin = listen.admin_addr().map(|addr| ("listen.admin", addr));
    for (key, addr) in std::iter::once(("listen.client_api", listen.client_api)).chain(admin) {
        TcpListener::bind(addr).with_context(|| format!("{key}: cannot bind {addr}"))?;
    }
    Ok(())
}

/// The database is reachable over a read-only connection, and its schema is
/// not newer than this binary. A missing SQLite file is fine (created on
/// first start); pending migrations are reported, not applied.
///
/// PgBouncer in transaction-pool mode rejects the read-only startup parameter
/// (`default_transaction_read_only`) unless it is listed in
/// `ignore_startup_parameters`: this check then fails while `serve` works.
pub fn database(loaded: &Loaded) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("cannot start the async runtime")?;
    runtime.block_on(database_status(&loaded.config))
}

async fn database_status(config: &Bootstrap) -> anyhow::Result<()> {
    let url = database::url(config)?;
    if database::is_in_memory(&url) {
        return note("database.url is an in-memory database; nothing to check");
    }
    if let Some(file) = deskhaven_db::sqlite_file(&url)
        && !file.exists()
    {
        return note(&format!(
            "database {} does not exist yet; it is created on first start",
            file.display()
        ));
    }
    let mut options = database::options(config, url);
    options.read_only = true;
    options.max_connections = 1;
    options.connect_timeout = CHECK_CONNECT_TIMEOUT;
    let db = database::connect(&options).await?;
    let status = deskhaven_db::migration_status(&db).await;
    db.close().await.context("cannot close the database")?;
    let status = status?;
    if !status.unknown.is_empty() {
        bail!(
            "the database has migrations unknown to this binary ({}); it was upgraded by a newer deskhaven",
            status.unknown.join(", ")
        );
    }
    if !status.pending.is_empty() {
        note(&format!(
            "{} pending migration(s) will be applied on the next start",
            status.pending.len()
        ))?;
    }
    Ok(())
}

fn note(text: &str) -> anyhow::Result<()> {
    writeln!(std::io::stderr().lock(), "note: {text}")?;
    Ok(())
}
