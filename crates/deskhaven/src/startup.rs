//! Startup steps (02 §7) up to bound listeners. Each failure aborts startup;
//! warnings are logged and startup continues.

use std::sync::Arc;
use std::time::Duration;

use deskhaven_config::bootstrap::Bootstrap;
use deskhaven_config::settings::PinSet;
use deskhaven_core::auth::limiter::LoginThrottle;
use deskhaven_core::clock::{SharedClock, SystemClock};
use deskhaven_core::crypto::SecretBox;
use deskhaven_core::keys::{
    KeyWarning, SignalKeyPaths, load_or_generate_master_key, load_or_generate_signal_key,
};
use deskhaven_core::sessions::SessionService;
use deskhaven_core::settings::SettingsService;
use deskhaven_core::state::AppState;
use deskhaven_core::users::{PasswordHasher, UserService};
use deskhaven_http::{HttpConfig, Listeners, NofileInfo, raise_nofile_limit};

use crate::instance_lock::{self, InstanceLock};
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
    signal_public_key: String,
}

/// Everything needed to serve.
pub struct Started {
    pub state: Arc<AppState>,
    pub listeners: Listeners,
    pub http: HttpConfig,
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

    // M1-06 hands the pair itself to the signal server; until then only the
    // public key is kept, so the private key leaves memory here.
    let signal = load_or_generate_signal_key(&SignalKeyPaths::from_bootstrap(config))?;
    log_key_warnings(&signal.warnings);
    let signal_public_key = signal.value.public_key_base64();
    tracing::info!(public_key = %signal_public_key, "hbbs key pair loaded");

    Ok(Prepared {
        lock,
        pins,
        crypto,
        signal_public_key,
    })
}

/// Listeners first (an instance that cannot bind never touches the
/// database), then database, migrations, settings. Until serving starts,
/// connections wait in the listen backlog.
pub async fn start(
    config: &Bootstrap,
    prepared: Prepared,
    drain_timeout: Duration,
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
        signal_public_key,
    } = prepared;
    let state = Arc::new(open_state(config, pins, crypto, signal_public_key).await?);
    Ok(Started {
        state,
        listeners,
        http,
        lock,
    })
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

    let clock: SharedClock = Arc::new(SystemClock);
    let users = UserService::with_clock(
        db.clone(),
        settings.clone(),
        PasswordHasher::default(),
        Arc::clone(&clock),
    );
    let login_throttle = LoginThrottle::new(db.clone(), settings.clone(), Arc::clone(&clock));
    let sessions = SessionService::new(db.clone(), settings.clone(), users.clone(), clock);

    Ok(AppState {
        db,
        settings,
        crypto,
        signal_public_key,
        users,
        sessions,
        login_throttle,
    })
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
