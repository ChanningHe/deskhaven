//! First-run setup link (05 §2.4): while the instance has no owner, startup logs a
//! one-time link to the console's `/setup` page once; `deskhaven admin create-setup-link`
//! issues a new one (retiring the old) when it was lost or expired.
//!
//! The token travels in the URL fragment (`/setup#token=`), which browsers never send to
//! a server, so it stays out of reverse-proxy access logs and `Referer` headers. No
//! password is ever printed: the owner chooses one on the page.

use std::io::Write;

use anyhow::Context;
use deskhaven_config::bootstrap::{Bootstrap, LoadOptions};
use deskhaven_core::clock::{Clock, SystemClock};
use deskhaven_core::setup::{self, SetupError, SetupService, SetupToken, StartupSetup};

use crate::{config_cmd, database};

/// Path of the console's setup page.
pub const SETUP_PATH: &str = "/setup";

/// The link for `token`: on the console's public URL when one is configured
/// (`admin_public_url` with a split admin listener, else `public_url`), else just the
/// path, to open on whatever address reaches the console.
pub fn link(config: &Bootstrap, token: &str) -> String {
    let base = if config.listen.admin_addr().is_some() {
        config.listen.admin_public_url.as_str()
    } else {
        config.public_url.as_str()
    };
    format!(
        "{}{SETUP_PATH}#token={token}",
        base.trim().trim_end_matches('/')
    )
}

/// Startup: logs the setup state; issues and logs a link when none is live.
pub async fn announce(config: &Bootstrap, setup: &SetupService) -> anyhow::Result<()> {
    match setup
        .on_startup()
        .await
        .context("cannot prepare the first-run setup")?
    {
        StartupSetup::Done => {}
        StartupSetup::Pending { expires_at } => tracing::warn!(
            expires_at = %expires_at.0,
            "first-run setup pending: open the setup link issued earlier, or run \
             `deskhaven admin create-setup-link` for a new one"
        ),
        StartupSetup::Issued(token) => log_issued(config, &token),
    }
    Ok(())
}

fn log_issued(config: &Bootstrap, token: &SetupToken) {
    tracing::warn!(
        link = %link(config, token.expose()),
        expires_at = %token.expires_at.0,
        "first-run setup: open this one-time link to create the owner account"
    );
}

/// `deskhaven admin create-setup-link`: issues a new link on the configured database and
/// prints it. Works while the server runs (it only writes `setup_tokens`).
pub fn create(options: &LoadOptions) -> anyhow::Result<()> {
    let loaded = config_cmd::load_reporting(options)?;
    let config = &loaded.config;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("cannot start the async runtime")?;
    let token = runtime.block_on(async {
        let url = database::url(config)?;
        let db = database::connect(&database::options(config, url)).await?;
        deskhaven_db::migrate(&db).await?;
        match setup::issue(&db, SystemClock.now_ms()).await {
            Ok(token) => Ok(token),
            Err(SetupError::OwnerExists) => Err(anyhow::anyhow!(
                "the instance already has an owner; the setup link is only for the first run"
            )),
            Err(err) => Err(anyhow::Error::new(err).context("cannot issue a setup link")),
        }
    })?;
    let mut out = std::io::stdout().lock();
    writeln!(out, "{}", link(config, token.expose()))?;
    writeln!(out, "valid until {} (single use)", token.expires_at.0)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use super::*;

    #[test]
    fn link_follows_the_console_url_and_keeps_the_token_in_the_fragment() {
        let mut config = Bootstrap::default();
        assert_eq!(link(&config, "tok"), "/setup#token=tok");

        config.public_url = "https://rd.example.com/".into();
        assert_eq!(
            link(&config, "tok"),
            "https://rd.example.com/setup#token=tok"
        );

        let admin: SocketAddr = "127.0.0.1:21113".parse().unwrap();
        config.listen.admin = Some(admin);
        config.listen.admin_public_url = "https://console.example.com".into();
        assert_eq!(
            link(&config, "tok"),
            "https://console.example.com/setup#token=tok"
        );
    }
}
