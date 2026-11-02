//! Process-wide state shared by the HTTP routers (02 §3). Built once at
//! startup (`deskhaven::startup::open_state`) and handed to the API crates
//! as `State<Arc<AppState>>`; later subsystems (presence, domain events)
//! become further fields.

use std::sync::Arc;

use sea_orm::DatabaseConnection;

use crate::crypto::SecretBox;
use crate::sessions::SessionService;
use crate::settings::SettingsService;
use crate::users::UserService;

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
}
