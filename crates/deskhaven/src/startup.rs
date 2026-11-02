//! Startup steps (02 §7) up to bound listeners. Each failure aborts startup;
//! warnings are logged and startup continues.

use std::sync::Arc;
use std::time::Duration;

use deskhaven_config::bootstrap::Bootstrap;
use deskhaven_config::settings::PinSet;
use deskhaven_core::crypto::SecretBox;
use deskhaven_core::keys::{
    KeyWarning, SignalKeyPaths, load_or_generate_master_key, load_or_generate_signal_key,
};
use deskhaven_core::settings::SettingsService;
use deskhaven_http::{HttpConfig, Listeners, NofileInfo, raise_nofile_limit};

use crate::state::AppState;
use crate::{data_dir, database, http};

/// Results of the synchronous steps, consumed by [`start`].
pub struct Prepared {
    pins: PinSet,
    crypto: Arc<SecretBox>,
    signal_public_key: String,
}

/// Everything needed to serve.
pub struct Started {
    pub state: Arc<AppState>,
    pub listeners: Listeners,
    pub http: HttpConfig,
}

/// Steps that need no runtime: pins (first, so a rejected configuration
/// writes nothing), the descriptor limit, data_dir, key material.
pub fn prepare(config: &Bootstrap) -> anyhow::Result<Prepared> {
    let pins = PinSet::from_bootstrap(&config.settings)?;
    log_nofile(raise_nofile_limit());
    if let Some(loose) = data_dir::prepare(&config.data_dir)? {
        tracing::warn!("{loose}");
    }

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
        pins,
        crypto,
        signal_public_key,
    })
}

/// Database, migrations, settings, listeners.
pub async fn start(
    config: &Bootstrap,
    prepared: Prepared,
    drain_timeout: Duration,
) -> anyhow::Result<Started> {
    let url = database::url(config)?;
    let backend = database::backend(&url);
    let db = database::connect(&database::options(config, url)).await?;
    tracing::info!(backend, "database connected");
    deskhaven_db::migrate(&db).await?;
    tracing::info!("database migrations applied");

    let (settings, warnings) =
        SettingsService::load(db.clone(), Arc::clone(&prepared.crypto), prepared.pins).await?;
    for warning in &warnings {
        tracing::warn!("{warning}");
    }

    let http = http::http_config(config, drain_timeout);
    http.validate()?;
    let listeners = Listeners::bind(&http::listen_config(&config.listen)).await?;
    let client_api = listeners.client_api_addr();
    tracing::info!(listener = "client_api", addr = %client_api, "listening");
    if let Some(admin) = listeners.admin_addr() {
        tracing::info!(listener = "admin", addr = %admin, "listening");
    }

    let state = Arc::new(AppState {
        db,
        settings,
        crypto: prepared.crypto,
        signal_public_key: prepared.signal_public_key,
    });
    Ok(Started {
        state,
        listeners,
        http,
    })
}

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
