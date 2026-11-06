//! `deskhaven-loadgen`: run with `cargo xtask loadgen [args]`, which builds it and the server
//! in release mode first. Exit status: 0 every NFR passed, 1 error, 2 an NFR failed.

use std::io::Write as _;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;
use deskhaven_loadgen::plan::{Rates, Timing};
use deskhaven_loadgen::server::ServerConfig;
use deskhaven_loadgen::{Config, LoadgenError, run};

/// Env var through which `cargo xtask loadgen` passes its own command line for the report.
const INVOCATION_ENV: &str = "DESKHAVEN_LOADGEN_INVOCATION";
const NFR_FAILED: u8 = 2;

/// Simulates RustDesk devices against a fresh `deskhaven serve` and reports the NFRs.
#[derive(Debug, Parser)]
#[command(name = "deskhaven-loadgen")]
struct Args {
    /// The deskhaven binary to start.
    #[arg(long)]
    server_bin: PathBuf,
    /// New or empty directory for the server's config, data and `server.log`.
    #[arg(long)]
    work_dir: PathBuf,
    /// Empty database for the server; default: SQLite in the work dir.
    #[arg(long)]
    database_url: Option<String>,
    #[arg(long, default_value_t = 10_000)]
    devices: usize,
    #[arg(long, value_parser = humantime::parse_duration, default_value = "10s")]
    idle: Duration,
    #[arg(long, value_parser = humantime::parse_duration, default_value = "60s")]
    ramp: Duration,
    #[arg(long, value_parser = humantime::parse_duration, default_value = "30s")]
    settle: Duration,
    /// Measurement window; keep it at least two `--last-seen-flush` intervals.
    #[arg(long, value_parser = humantime::parse_duration, default_value = "180s")]
    steady: Duration,
    #[arg(long, value_parser = humantime::parse_duration, default_value = "3s")]
    tick: Duration,
    #[arg(long, value_parser = humantime::parse_duration, default_value = "15s")]
    heartbeat_interval: Duration,
    #[arg(long, value_parser = humantime::parse_duration, default_value = "12s")]
    register_interval: Duration,
    /// Pinned `devices.last_seen_flush` (default: the server default, 60 s).
    #[arg(long, value_parser = humantime::parse_duration, default_value = "60s")]
    last_seen_flush: Duration,
    #[arg(long, default_value_t = 1.0)]
    connections_per_device_hour: f64,
    #[arg(long, default_value_t = 1.0)]
    sysinfo_per_device_hour: f64,
    #[arg(long, default_value_t = 31114)]
    api_port: u16,
    #[arg(long, default_value_t = 31116)]
    signal_port: u16,
    #[arg(long, default_value = "127.64.0.1")]
    first_ip: Ipv4Addr,
    #[arg(long, default_value = "info")]
    server_log_level: String,
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// Also write the Markdown report here.
    #[arg(long)]
    out: Option<PathBuf>,
}

impl Args {
    fn into_config(self, command: String) -> (Config, Option<PathBuf>) {
        let server = ServerConfig {
            bin: self.server_bin,
            work_dir: self.work_dir,
            database_url: self.database_url,
            api_port: self.api_port,
            signal_port: self.signal_port,
            last_seen_flush: self.last_seen_flush,
            log_level: self.server_log_level,
        };
        let timing = Timing {
            tick: self.tick,
            heartbeat: self.heartbeat_interval,
            register: self.register_interval,
            ..Timing::default()
        };
        let rates = Rates {
            connections_per_hour: self.connections_per_device_hour,
            sysinfo_per_hour: self.sysinfo_per_device_hour,
        };
        let config = Config {
            server,
            devices: self.devices,
            first_ip: self.first_ip,
            timing,
            rates,
            idle: self.idle,
            ramp: self.ramp,
            settle: self.settle,
            steady: self.steady,
            seed: self.seed,
            command,
        };
        (config, self.out)
    }
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let command = std::env::var(INVOCATION_ENV)
        .unwrap_or_else(|_| std::env::args().collect::<Vec<_>>().join(" "));
    let (config, out) = Args::parse().into_config(command);
    match execute(&config, out) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(NFR_FAILED),
        Err(err) => {
            tracing::error!(error = %error_chain(&err), "load test failed");
            ExitCode::FAILURE
        }
    }
}

/// `Ok(passed)` once the report is written.
fn execute(config: &Config, out: Option<PathBuf>) -> Result<bool, LoadgenError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(LoadgenError::Runtime)?;
    let report = runtime.block_on(run(config))?;
    let text = report.to_string();
    if let Some(path) = out {
        std::fs::write(&path, &text).map_err(|source| LoadgenError::Io {
            action: "write",
            path,
            source,
        })?;
    }
    let stdout_path = PathBuf::from("<stdout>");
    std::io::stdout()
        .lock()
        .write_all(text.as_bytes())
        .map_err(|source| LoadgenError::Io {
            action: "write",
            path: stdout_path,
            source,
        })?;
    Ok(report.passed())
}

fn error_chain(err: &dyn std::error::Error) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}
