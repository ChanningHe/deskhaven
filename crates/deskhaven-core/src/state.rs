//! Process-wide state shared by the HTTP routers (02 §3). Built once at
//! startup (`deskhaven::startup::open_state`) and handed to the API crates
//! as `State<Arc<AppState>>`; later subsystems (domain events)
//! become further fields.

use std::sync::Arc;

use sea_orm::DatabaseConnection;

use crate::auth::limiter::LoginThrottle;
use crate::clock::SharedClock;
use crate::console_audit::ConsoleAuditLog;
use crate::crypto::SecretBox;
use crate::devices::DeviceService;
use crate::groups::GroupService;
use crate::login_events::LoginEventLog;
use crate::oidc::{OidcService, ReqwestFetch};
use crate::sessions::SessionService;
use crate::settings::SettingsService;
use crate::setup::SetupService;
use crate::strategies::StrategyService;
use crate::tfa::TfaService;
use crate::users::{PasswordHasher, UserService};

pub struct AppState {
    /// Migrated connection pool (cheap to clone).
    pub db: DatabaseConnection,
    /// Effective runtime settings with hot reload.
    pub settings: SettingsService,
    /// Master-key cipher for secrets at rest.
    pub crypto: Arc<SecretBox>,
    /// hbbs public key (base64), the value clients enter as "Key". Public data.
    pub signal_public_key: String,
    /// User accounts and local passwords.
    pub users: UserService,
    /// Client and console sessions, with the validated-session cache.
    pub sessions: SessionService,
    /// Per-IP rate limit and account lockout for login-type endpoints.
    pub login_throttle: LoginThrottle,
    /// Audit trail of login attempts.
    pub login_events: LoginEventLog,
    /// TOTP enrolment, recovery codes and two-step login challenges.
    pub tfa: TfaService,
    /// Audit trail of console / API / CLI changes.
    pub console_audit: ConsoleAuditLog,
    /// OIDC providers and logins.
    pub oidc: OidcService,
    /// User groups (membership changes invalidate the strategy cache).
    pub groups: GroupService,
    /// Strategies and the effective-strategy cache, watching `users` and `groups`.
    pub strategies: StrategyService,
    /// Device registry operations and the presence table (`devices.presence()`), which the
    /// signal server's liveness notifications feed.
    pub devices: DeviceService,
    /// First-run owner setup.
    pub setup: SetupService,
    /// See [`StateConfig::console_url`].
    pub console_url: String,
}

/// Bootstrap values the services need beyond the database and the settings.
#[derive(Debug, Clone, Default)]
pub struct StateConfig {
    /// `public_url`, as validated by the bootstrap loader; blank disables OIDC logins.
    pub public_url: String,
    /// Base URL of the web console as reached from the client API listener, for redirects
    /// from there into the console (the OIDC callback of a console login):
    /// `listen.admin_public_url` when the admin side has its own listener, blank when the
    /// console shares the client API listener (redirects stay relative).
    pub console_url: String,
}

impl AppState {
    /// Wires the services around a migrated `db` and loaded `settings`, all on one `clock`
    /// (production: `SystemClock`, `PasswordHasher::default()`). Must be called inside a
    /// Tokio runtime (the session service spawns its event consumer).
    pub fn new(
        db: DatabaseConnection,
        settings: SettingsService,
        crypto: Arc<SecretBox>,
        signal_public_key: String,
        hasher: PasswordHasher,
        clock: SharedClock,
        config: &StateConfig,
    ) -> Self {
        let users =
            UserService::with_clock(db.clone(), settings.clone(), hasher, Arc::clone(&clock));
        let login_throttle = LoginThrottle::new(db.clone(), settings.clone(), Arc::clone(&clock));
        let login_events = LoginEventLog::new(db.clone(), Arc::clone(&clock));
        let console_audit = ConsoleAuditLog::new(db.clone(), Arc::clone(&clock));
        let tfa = TfaService::new(
            db.clone(),
            Arc::clone(&crypto),
            settings.clone(),
            Arc::clone(&clock),
        );
        let oidc = OidcService::new(
            db.clone(),
            Arc::clone(&crypto),
            settings.clone(),
            users.clone(),
            Arc::clone(&clock),
            &config.public_url,
            Arc::new(ReqwestFetch::new()),
        );
        let groups = GroupService::new(db.clone());
        let strategies = StrategyService::new(db.clone(), Arc::clone(&clock));
        strategies.watch_users(&users);
        strategies.watch_groups(&groups);
        let devices = DeviceService::new(
            db.clone(),
            settings.clone(),
            Arc::clone(&crypto),
            strategies.clone(),
            Arc::clone(&clock),
        );
        let setup = SetupService::new(db.clone(), users.clone(), Arc::clone(&clock));
        let sessions = SessionService::new(db.clone(), settings.clone(), users.clone(), clock);
        Self {
            db,
            settings,
            crypto,
            signal_public_key,
            users,
            sessions,
            login_throttle,
            login_events,
            tfa,
            console_audit,
            oidc,
            groups,
            strategies,
            devices,
            setup,
            console_url: config.console_url.trim().trim_end_matches('/').to_owned(),
        }
    }
}
