//! Process logger: `tracing-subscriber` on stderr, filtered by `log.level`,
//! human-readable or JSON per `log.format`. The `tracing-log` bridge forwards
//! `log` records (hbb_common, sqlx) into the same pipeline; flexi_logger is
//! never initialized (02 §5.1). Plain text without ANSI colors: logs mostly
//! end up in journald or container logs (and the `ansi` feature would leak
//! into every crate's log-capturing tests through feature unification).

use anyhow::anyhow;
use deskhaven_config::bootstrap::{Log, LogFormat};
use tracing_subscriber::EnvFilter;

/// Installs the global subscriber (and the `log` bridge). Call once.
pub fn init(log: &Log) -> anyhow::Result<()> {
    // Already validated with the bootstrap; the message never echoes the filter.
    let filter = EnvFilter::builder()
        .parse(&log.level)
        .map_err(|_| anyhow!("log.level: not a valid tracing filter"))?;
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr);
    let installed = match log.format {
        LogFormat::Pretty => builder.try_init(),
        LogFormat::Json => builder.json().try_init(),
    };
    installed.map_err(|err| anyhow!("cannot install the logger: {err}"))
}
