use std::sync::Arc;

use deskhaven_config::bootstrap::Signal;
use sea_orm::DatabaseConnection;

use crate::api_tokens::ApiTokenService;
use crate::audit::AuditService;
use crate::auth::limiter::LoginThrottle;
use crate::clock::SharedClock;
use crate::console_audit::ConsoleAuditLog;
use crate::crypto::SecretBox;
use crate::devices::DeviceService;
use crate::groups::GroupService;
use crate::ldap::{Ldap3Connector, LdapService};
use crate::login_events::LoginEventLog;
use crate::oidc::{HttpFetch, OidcService, ReqwestFetch};
use crate::sessions::SessionService;
use crate::settings::SettingsService;
use crate::setup::SetupService;
use crate::strategies::StrategyService;
use crate::tfa::TfaService;
use crate::update::UpdateService;
use crate::users::{PasswordHasher, UserService};
use crate::web_client::WebClientService;

pub struct AppState {
    pub db: DatabaseConnection,
    pub settings: SettingsService,
    pub clock: SharedClock,
    pub crypto: Arc<SecretBox>,
    pub signal_public_key: String,
    pub users: UserService,
    pub api_tokens: ApiTokenService,
    pub sessions: SessionService,
    pub login_throttle: LoginThrottle,
    pub login_events: LoginEventLog,
    pub tfa: TfaService,
    pub console_audit: ConsoleAuditLog,
    pub oidc: OidcService,
    pub ldap: LdapService,
    /// The one instance everything shares, so the strategy cache's commit hooks fire for every
    /// membership change.
    pub groups: GroupService,
    pub strategies: StrategyService,
    pub devices: DeviceService,
    pub audit: AuditService,
    pub setup: SetupService,
    pub updates: Arc<UpdateService>,
    pub web_client: WebClientService,
    pub console_url: String,
    pub public_url: String,
    pub signal_port: u16,
}

#[derive(Debug, Clone)]
pub struct StateConfig {
    pub public_url: String,
    /// Blank when the console shares the client API listener (redirects stay relative).
    pub console_url: String,
    pub signal_port: u16,
}

impl Default for StateConfig {
    fn default() -> Self {
        Self {
            public_url: String::new(),
            console_url: String::new(),
            signal_port: Signal::default().port,
        }
    }
}

impl AppState {
    /// Must be called inside a Tokio runtime (the session service spawns its event consumer).
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
            users.clone(),
        );
        let fetch: Arc<dyn HttpFetch> = Arc::new(ReqwestFetch::new());
        let oidc = OidcService::new(
            db.clone(),
            Arc::clone(&crypto),
            settings.clone(),
            users.clone(),
            Arc::clone(&clock),
            &config.public_url,
            Arc::clone(&fetch),
        );
        let updates = Arc::new(UpdateService::new(
            settings.clone(),
            Arc::clone(&clock),
            fetch,
        ));
        let groups = GroupService::new(db.clone());
        let ldap = LdapService::new(
            db.clone(),
            Arc::clone(&crypto),
            users.clone(),
            groups.clone(),
            Arc::clone(&clock),
            Arc::new(Ldap3Connector::new()),
        );
        let strategies = StrategyService::new(db.clone(), Arc::clone(&crypto), Arc::clone(&clock));
        strategies.watch_users(&users);
        strategies.watch_groups(&groups);
        let devices = DeviceService::new(
            db.clone(),
            settings.clone(),
            Arc::clone(&crypto),
            strategies.clone(),
            Arc::clone(&clock),
        );
        let audit = AuditService::new(db.clone(), devices.clone(), Arc::clone(&clock));
        let setup = SetupService::new(db.clone(), users.clone(), Arc::clone(&clock));
        let api_tokens = ApiTokenService::new(db.clone(), Arc::clone(&clock));
        let web_client = WebClientService::new(db.clone(), settings.clone(), Arc::clone(&clock));
        let sessions = SessionService::new(
            db.clone(),
            settings.clone(),
            users.clone(),
            Arc::clone(&clock),
        );
        Self {
            db,
            settings,
            clock,
            crypto,
            signal_public_key,
            users,
            api_tokens,
            sessions,
            login_throttle,
            login_events,
            tfa,
            console_audit,
            oidc,
            ldap,
            groups,
            strategies,
            devices,
            audit,
            setup,
            updates,
            web_client,
            console_url: config.console_url.trim().trim_end_matches('/').to_owned(),
            public_url: config.public_url.trim().to_owned(),
            signal_port: config.signal_port,
        }
    }
}
