//! Startup steps (02 §7) up to bound listeners. Each failure aborts startup;
//! warnings are logged and startup continues.

use std::sync::Arc;
use std::time::Duration;

use deskhaven_config::bootstrap::Bootstrap;
use deskhaven_config::settings::PinSet;
use deskhaven_core::clock::SystemClock;
use deskhaven_core::crypto::SecretBox;
use deskhaven_core::devices::DeviceRepository;
use deskhaven_core::keys::{
    KeyWarning, SignalKeyPair, SignalKeyPaths, load_or_generate_master_key,
    load_or_generate_signal_key,
};
use deskhaven_core::settings::SettingsService;
use deskhaven_core::state::AppState;
use deskhaven_core::users::PasswordHasher;
use deskhaven_http::{HttpConfig, Listeners, NofileInfo, raise_nofile_limit};
use deskhaven_signal::AllowAll;
use tokio_util::sync::CancellationToken;

use crate::instance_lock::{self, InstanceLock};
use crate::signal_server::{self, Backoff, Launcher, RunningSignal};
use crate::signal_store::SignalPeerStore;
use crate::{data_dir, database, http};

/// Files this process creates (data_dir, database and its side files, keys,
/// lock file) are owner-only.
#[cfg(unix)]
const UMASK: u32 = 0o077;

/// Results of the synchronous steps, consumed by [`start`].
pub struct Prepared {
    lock: InstanceLock,
    pins: PinSet,
    crypto: Arc<SecretBox>,
    signal_key: SignalKeyPair,
}

/// Everything needed to serve.
pub struct Started {
    pub state: Arc<AppState>,
    pub listeners: Listeners,
    pub http: HttpConfig,
    /// The supervised signal server; `None` with `[signal] enabled = false`.
    pub signal: Option<RunningSignal>,
    /// Held until the process exits.
    pub lock: InstanceLock,
}

/// Steps that need no runtime: pins (first, so a rejected configuration
/// writes nothing), umask, the descriptor limit, data_dir, the instance lock,
/// key material.
pub fn prepare(config: &Bootstrap) -> anyhow::Result<Prepared> {
    let pins = PinSet::from_bootstrap(&config.settings)?;
    restrict_umask();
    log_nofile(raise_nofile_limit());
    if let Some(loose) = data_dir::prepare(&config.data_dir)? {
        tracing::warn!("{loose}");
    }
    let lock = instance_lock::acquire(&config.data_dir)?;

    let master = load_or_generate_master_key(&config.security.master_key, &config.data_dir)?;
    log_key_warnings(&master.warnings);
    let crypto = Arc::new(SecretBox::new(&master.value));

    // Loaded even with the signal server disabled: the client API serves the
    // public key, and `key show` must agree with a later start.
    let signal = load_or_generate_signal_key(&SignalKeyPaths::from_bootstrap(config))?;
    log_key_warnings(&signal.warnings);
    tracing::info!(public_key = %signal.value.public_key_base64(), "hbbs key pair loaded");

    Ok(Prepared {
        lock,
        pins,
        crypto,
        signal_key: signal.value,
    })
}

/// Listeners first (an instance that cannot bind never touches the
/// database), then database, migrations, settings, then the signal server
/// (which loads the peer registry before binding its ports). Until serving
/// starts, HTTP connections wait in the listen backlog. The signal server
/// stops when `token` is cancelled.
pub async fn start(
    config: &Bootstrap,
    prepared: Prepared,
    drain_timeout: Duration,
    token: &CancellationToken,
) -> anyhow::Result<Started> {
    let http = http::http_config(config, drain_timeout);
    http.validate()?;
    let listeners = Listeners::bind(&http::listen_config(&config.listen)).await?;
    let client_api = listeners.client_api_addr();
    tracing::info!(listener = "client_api", addr = %client_api, "listening");
    if let Some(admin) = listeners.admin_addr() {
        tracing::info!(listener = "admin", addr = %admin, "listening");
    }

    let Prepared {
        lock,
        pins,
        crypto,
        signal_key,
    } = prepared;
    let public_key = signal_key.public_key_base64();
    let state = Arc::new(open_state(config, pins, crypto, public_key).await?);
    let signal = start_signal(config, &signal_key, &state, token).await?;
    Ok(Started {
        state,
        listeners,
        http,
        signal,
        lock,
    })
}

/// The signal server over the device registry, following the settings;
/// `None` when disabled.
async fn start_signal(
    config: &Bootstrap,
    key: &SignalKeyPair,
    state: &AppState,
    token: &CancellationToken,
) -> anyhow::Result<Option<RunningSignal>> {
    if !config.signal.enabled {
        tracing::info!("signal server disabled");
        return Ok(None);
    }
    let launcher = Launcher::new(
        signal_server::base_config(&config.signal, &config.security, key)?,
        Arc::new(SignalPeerStore::new(DeviceRepository::new(
            state.db.clone(),
        ))),
        Arc::new(AllowAll::new()),
        state.settings.subscribe(),
    );
    let running = signal_server::start(launcher, Backoff::DEFAULT, token).await?;
    Ok(Some(running))
}

/// Database, migrations, settings: the [`AppState`] the routers serve.
/// Also the entry point of in-process harnesses (deskhaven-contract), which
/// build the state without listeners, umask or instance lock.
pub async fn open_state(
    config: &Bootstrap,
    pins: PinSet,
    crypto: Arc<SecretBox>,
    signal_public_key: String,
) -> anyhow::Result<AppState> {
    let url = database::url(config)?;
    let backend = database::backend(&url);
    let db = database::connect(&database::options(config, url)).await?;
    tracing::info!(backend, "database connected");
    deskhaven_db::migrate(&db).await?;
    tracing::info!("database migrations applied");

    let (settings, warnings) = SettingsService::load(db.clone(), Arc::clone(&crypto), pins).await?;
    for warning in &warnings {
        tracing::warn!("{warning}");
    }

    Ok(AppState::new(
        db,
        settings,
        crypto,
        signal_public_key,
        PasswordHasher::default(),
        Arc::new(SystemClock),
    ))
}

#[cfg(unix)]
fn restrict_umask() {
    let previous = rustix::process::umask(rustix::fs::Mode::from_raw_mode(UMASK));
    tracing::debug!(
        previous = format!("{:04o}", previous.bits()),
        "umask set to 0077"
    );
}

#[cfg(not(unix))]
fn restrict_umask() {}

fn log_nofile(info: NofileInfo) {
    tracing::info!(
        soft = ?info.soft,
        hard = ?info.hard,
        raised = info.raised,
        "RLIMIT_NOFILE"
    );
}

fn log_key_warnings(warnings: &[KeyWarning]) {
    for warning in warnings {
        tracing::warn!("{warning}");
    }
}
