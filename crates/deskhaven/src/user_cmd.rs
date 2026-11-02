//! `deskhaven user ...` subcommands: account maintenance on the configured database,
//! safe while the server runs (the lockout lives in the database, not in memory).

use std::io::Write;
use std::sync::Arc;

use anyhow::{Context, bail};
use deskhaven_config::bootstrap::{Bootstrap, LoadOptions};
use deskhaven_core::auth::limiter::{Unlocked, unlock_guarded};
use deskhaven_core::clock::SystemClock;
use deskhaven_core::console_audit::{Actor, ConsoleAuditLog, NewConsoleAudit, TxGuard};
use deskhaven_core::users::username_norm;
use serde_json::json;

use crate::{config_cmd, database};

/// Clears the login lockout and failure counters of `username` (06 §3.6), whether or not
/// such an account exists, and records it in the console audit (actor `cli`).
pub fn unlock(options: &LoadOptions, username: &str) -> anyhow::Result<()> {
    if username_norm(username).is_none() {
        bail!("{username:?} is not a valid username (1-64 characters of A-Z a-z 0-9 . _ @ -)");
    }
    let loaded = config_cmd::load_reporting(options)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("cannot start the async runtime")?;
    let cleared = runtime.block_on(unlock_in_database(&loaded.config, username))?;
    writeln!(
        std::io::stdout().lock(),
        "unlocked {username}: {cleared} lockout entries cleared"
    )?;
    Ok(())
}

async fn unlock_in_database(config: &Bootstrap, username: &str) -> anyhow::Result<u64> {
    let url = database::url(config)?;
    if database::is_in_memory(&url) {
        bail!("database.url is an in-memory database; there is nothing to unlock");
    }
    if let Some(file) = deskhaven_db::sqlite_file(&url)
        && !file.exists()
    {
        bail!("database {} does not exist yet", file.display());
    }
    let mut options = database::options(config, url);
    options.max_connections = 1;
    let db = database::connect(&options).await?;
    let result = unlock_on(&db, username).await;
    db.close().await.context("cannot close the database")?;
    result
}

async fn unlock_on(db: &sea_orm::DatabaseConnection, username: &str) -> anyhow::Result<u64> {
    let status = deskhaven_db::migration_status(db).await?;
    if !status.is_current() {
        bail!(
            "the database schema does not match this binary ({} pending, {} unknown migrations); \
             run the matching deskhaven version",
            status.pending.len(),
            status.unknown.len()
        );
    }
    let log = ConsoleAuditLog::new(db.clone(), Arc::new(SystemClock));
    let name = username.to_owned();
    let guard = TxGuard::new(&log, move |_, after: Option<&Unlocked>| {
        let cleared = after.map_or(0, |u| u.cleared);
        Ok(NewConsoleAudit {
            actor: Actor::Cli,
            action: "user.unlock".to_owned(),
            target_type: "username".to_owned(),
            target_id: name.clone(),
            before: None,
            after: Some(json!({ "username": name, "lockout_entries_cleared": cleared })),
            ip: None,
        })
    });
    unlock_guarded(db, username, guard)
        .await
        .context("cannot clear the lockout")
}
