//! The server under test: `deskhaven serve` in its own working directory, started and
//! stopped by the load generator (so its pid, ports and settings are known).

use std::fs::File;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use hyper::Method;
use tokio::time::{Instant, sleep};

use crate::LoadgenError;
use crate::http;

const LOCALHOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const POLL: Duration = Duration::from_millis(100);
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
/// The server's own shutdown deadline is 10 s (02 §7).
const STOP_LIMIT: Duration = Duration::from_secs(15);
const CONFIG_FILE: &str = "deskhaven.toml";
const RESTART_LOG: &str = "server-restart.log";
#[cfg(unix)]
const DATA_DIR_MODE: u32 = 0o700;

/// How to run the server.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub bin: PathBuf,
    /// Holds `deskhaven.toml`, `data/` and `server.log`.
    pub work_dir: PathBuf,
    /// `None`: the default SQLite database in `data/`.
    pub database_url: Option<String>,
    pub api_port: u16,
    pub signal_port: u16,
    /// Pinned `devices.last_seen_flush`, whole seconds.
    pub last_seen_flush: Duration,
    pub log_level: String,
}

impl ServerConfig {
    pub fn api(&self) -> SocketAddr {
        SocketAddr::new(LOCALHOST, self.api_port)
    }

    pub fn signal(&self) -> SocketAddr {
        SocketAddr::new(LOCALHOST, self.signal_port)
    }

    pub fn log_path(&self) -> PathBuf {
        self.work_dir.join("server.log")
    }

    /// The URL the server uses, the default SQLite file included.
    pub fn effective_database_url(&self) -> String {
        self.database_url.clone().unwrap_or_else(|| {
            let file = self.work_dir.join("data").join("deskhaven.db");
            format!("sqlite://{}", file.display())
        })
    }

    /// `deskhaven.toml` for this run.
    pub fn render(&self) -> String {
        let mut root = toml::Table::new();
        let data_dir = self.work_dir.join("data").display().to_string();
        root.insert("data_dir".into(), data_dir.into());
        if let Some(url) = &self.database_url {
            root.insert("database".into(), table([("url", url.clone().into())]));
        }
        let api = self.api().to_string();
        root.insert("listen".into(), table([("client_api", api.into())]));
        let port = i64::from(self.signal_port);
        root.insert("signal".into(), table([("port", port.into())]));
        let level = self.log_level.clone();
        let log = table([("format", "json".into()), ("level", level.into())]);
        root.insert("log".into(), log);
        let flush = format!("{}s", self.last_seen_flush.as_secs());
        let settings = table([("devices.last_seen_flush", flush.into())]);
        root.insert("settings".into(), settings);
        root.to_string()
    }

    /// The same with absolute paths, since the server runs inside the work dir. A bare
    /// program name (no `/`) stays as it is: it is looked up on `PATH`.
    pub fn absolute(&self) -> Result<Self, LoadgenError> {
        let resolve = |path: &Path| std::path::absolute(path).map_err(io_error("resolve", path));
        let bin = match self.bin.components().count() > 1 {
            true => resolve(&self.bin)?,
            false => self.bin.clone(),
        };
        Ok(Self {
            bin,
            work_dir: resolve(&self.work_dir)?,
            ..self.clone()
        })
    }
}

fn table<const N: usize>(entries: [(&str, toml::Value); N]) -> toml::Value {
    let table: toml::Table = entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect();
    toml::Value::Table(table)
}

/// A running server; killed on drop if still alive.
pub struct Server {
    child: Child,
    config: ServerConfig,
    /// Its stdout and stderr.
    log: PathBuf,
}

impl Server {
    /// Writes the configuration into the (new or empty) work dir and starts the server.
    pub fn spawn(config: &ServerConfig) -> Result<Self, LoadgenError> {
        let config = config.absolute()?;
        prepare_work_dir(&config.work_dir)?;
        write(&config.work_dir.join(CONFIG_FILE), &config.render())?;
        let log = config.log_path();
        Self::launch(config, log)
    }

    /// Starts the server again in the work dir of an earlier [`Server::spawn`], on the same
    /// configuration and database; it logs to `server-restart.log`.
    pub fn restart(config: &ServerConfig) -> Result<Self, LoadgenError> {
        let config = config.absolute()?;
        let log = config.work_dir.join(RESTART_LOG);
        Self::launch(config, log)
    }

    fn launch(config: ServerConfig, log: PathBuf) -> Result<Self, LoadgenError> {
        let out = File::create(&log).map_err(io_error("create", &log))?;
        let err = out.try_clone().map_err(io_error("open", &log))?;
        let child = Command::new(&config.bin)
            .arg("--config")
            .arg(config.work_dir.join(CONFIG_FILE))
            .arg("serve")
            .env_clear()
            .current_dir(&config.work_dir)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err)
            .spawn()
            .map_err(|source| LoadgenError::Spawn {
                path: config.bin.clone(),
                source,
            })?;
        tracing::info!(pid = child.id(), log = %log.display(), "server started");
        Ok(Self { child, config, log })
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// The configuration it runs with (absolute paths).
    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    /// Waits until `/readyz` answers 200 (database open, signal server serving).
    pub async fn wait_ready(&mut self, limit: Duration) -> Result<(), LoadgenError> {
        let deadline = Instant::now() + limit;
        let api = self.config.api();
        loop {
            let probe = http::send(LOCALHOST, api, Method::GET, "/readyz", None, PROBE_TIMEOUT);
            if probe.await.is_ok_and(|reply| reply.status == 200) {
                return Ok(());
            }
            self.check_alive()?;
            if Instant::now() >= deadline {
                let log = self.log.clone();
                return Err(LoadgenError::NotReady { limit, log });
            }
            sleep(POLL).await;
        }
    }

    fn check_alive(&mut self) -> Result<(), LoadgenError> {
        let pid = self.pid();
        match self.child.try_wait() {
            Ok(None) => Ok(()),
            Ok(Some(status)) => Err(LoadgenError::ServerExited {
                status,
                log: self.log.clone(),
            }),
            Err(source) => Err(LoadgenError::Stop { pid, source }),
        }
    }

    /// SIGTERM, then SIGKILL if it outlives [`STOP_LIMIT`]. `Ok(true)`: clean exit (status 0).
    pub async fn stop(mut self) -> Result<bool, LoadgenError> {
        let pid = self.pid();
        let stop_error = |source| LoadgenError::Stop { pid, source };
        terminate(&mut self.child).map_err(stop_error)?;
        let deadline = Instant::now() + STOP_LIMIT;
        loop {
            if let Some(status) = self.child.try_wait().map_err(stop_error)? {
                return Ok(status.success());
            }
            if Instant::now() >= deadline {
                tracing::warn!(pid, "server ignored SIGTERM; killing it");
                self.child.kill().map_err(stop_error)?;
                self.child.wait().map_err(stop_error)?;
                return Ok(false);
            }
            sleep(POLL).await;
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            // Last resort on an error path; nothing to report to.
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// SIGTERM: the server's graceful shutdown (final last-seen flush included).
#[cfg(unix)]
fn terminate(child: &mut Child) -> std::io::Result<()> {
    let raw = i32::try_from(child.id()).map_err(|_| std::io::Error::other("pid out of range"))?;
    if let Some(target) = rustix::process::Pid::from_raw(raw) {
        // ESRCH (already gone) shows up in the caller's `try_wait`.
        let _ = rustix::process::kill_process(target, rustix::process::Signal::TERM);
    }
    Ok(())
}

/// No signals: stop it outright (the report then shows an unclean shutdown).
#[cfg(not(unix))]
fn terminate(child: &mut Child) -> std::io::Result<()> {
    child.kill()
}

/// A fresh run needs a fresh directory: an old database would skew every write count.
fn prepare_work_dir(dir: &Path) -> Result<(), LoadgenError> {
    if dir.exists() {
        let mut entries = std::fs::read_dir(dir).map_err(io_error("read", dir))?;
        if entries.next().is_some() {
            return Err(LoadgenError::Config(format!(
                "work dir {} is not empty",
                dir.display()
            )));
        }
    }
    let data = dir.join("data");
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    // 0700, as the server asks of its data dir (keys and database).
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, DATA_DIR_MODE);
    builder.create(&data).map_err(io_error("create", &data))
}

fn write(path: &Path, text: &str) -> Result<(), LoadgenError> {
    std::fs::write(path, text).map_err(io_error("write", path))
}

fn io_error(action: &'static str, path: &Path) -> impl FnOnce(std::io::Error) -> LoadgenError {
    let path = path.to_path_buf();
    move |source| LoadgenError::Io {
        action,
        path,
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(database_url: Option<&str>) -> ServerConfig {
        ServerConfig {
            bin: PathBuf::from("deskhaven"),
            work_dir: PathBuf::from("/tmp/run \"1\""),
            database_url: database_url.map(str::to_owned),
            api_port: 31114,
            signal_port: 31116,
            last_seen_flush: Duration::from_secs(60),
            log_level: "info".to_owned(),
        }
    }

    #[test]
    fn rendered_config_pins_ports_flush_and_json_logs() {
        let parsed: toml::Table = config(None).render().parse().unwrap();

        assert_eq!(parsed["data_dir"].as_str(), Some("/tmp/run \"1\"/data"));
        assert_eq!(
            parsed["listen"]["client_api"].as_str(),
            Some("127.0.0.1:31114")
        );
        assert_eq!(parsed["signal"]["port"].as_integer(), Some(31116));
        assert_eq!(parsed["log"]["format"].as_str(), Some("json"));
        assert_eq!(
            parsed["settings"]["devices.last_seen_flush"].as_str(),
            Some("60s")
        );
        assert!(parsed.get("database").is_none());
    }

    #[test]
    fn paths_become_absolute_except_a_bare_program_name() {
        let relative = ServerConfig {
            bin: PathBuf::from("target/release/deskhaven"),
            work_dir: PathBuf::from("target/run"),
            ..config(None)
        };

        let resolved = relative.absolute().unwrap();

        assert!(resolved.bin.is_absolute() && resolved.bin.ends_with("target/release/deskhaven"));
        assert!(resolved.work_dir.is_absolute() && resolved.work_dir.ends_with("target/run"));
        assert_eq!(
            config(None).absolute().unwrap().bin,
            PathBuf::from("deskhaven")
        );
    }

    #[test]
    fn database_url_is_passed_through_or_defaults_to_the_data_dir() {
        let pg = config(Some("postgres://u:p@127.0.0.1:55404/db"));
        let parsed: toml::Table = pg.render().parse().unwrap();
        assert_eq!(
            parsed["database"]["url"].as_str(),
            Some("postgres://u:p@127.0.0.1:55404/db")
        );
        assert_eq!(
            config(None).effective_database_url(),
            "sqlite:///tmp/run \"1\"/data/deskhaven.db"
        );
    }
}
