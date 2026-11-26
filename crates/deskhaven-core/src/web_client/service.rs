use std::net::IpAddr;
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use base64_022::Engine as _;
use base64_022::engine::general_purpose::URL_SAFE_NO_PAD;
use deskhaven_config::settings::{
    WEB_CLIENT_ENABLED, WEB_CLIENT_ICE_SERVERS, WEB_CLIENT_TURN_CREDENTIAL,
    WEB_CLIENT_TURN_USERNAME, ice_urls, is_turn_url,
};
use sea_orm::{DatabaseConnection, DbErr};

use super::limits::{
    KeyedLimit, WEB_SESSIONS_PER_DEVICE_PER_MINUTE, WEB_SESSIONS_PER_USER_PER_MINUTE,
};
use super::port::{CandidateRejected, PunchError, PunchRequest, WebPunches};
use super::table::{Candidates, NewSession, Table};
use crate::clock::SharedClock;
use crate::crypto::{CryptoError, fill_random};
use crate::devices::{DeviceRepository, DeviceStatus};
use crate::settings::SettingsService;
use crate::users::Role;

const SESSION_ID_BYTES: usize = 16;

#[derive(Debug, thiserror::Error)]
pub enum WebClientError {
    #[error("the web client is disabled")]
    Disabled,
    #[error("the signal server is not available")]
    Unavailable,
    #[error("not found")]
    NotFound,
    #[error("too many web client sessions, try again later")]
    RateLimited(Duration),
    #[error("too many open web client sessions")]
    TooManySessions,
    #[error(transparent)]
    Punch(PunchError),
    #[error(transparent)]
    Candidate(CandidateRejected),
    #[error("the web client session is closed")]
    Closed,
    #[error(transparent)]
    Database(#[from] DbErr),
    #[error(transparent)]
    Crypto(#[from] CryptoError),
}

impl From<PunchError> for WebClientError {
    fn from(err: PunchError) -> Self {
        match err {
            PunchError::Stopped => Self::Unavailable,
            other => Self::Punch(other),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Opener {
    pub user_id: i64,
    pub role: Role,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenRequest {
    pub device_id: String,
    pub offer: Option<String>,
    pub client: IpAddr,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Opened {
    pub id: String,
    pub uuid: String,
    pub relay_server: String,
    pub signed_pk: Vec<u8>,
    pub answer: Option<String>,
}

impl std::fmt::Debug for Opened {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Opened")
            .field("relay_server", &self.relay_server)
            .field("answer", &self.answer.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct IceServer {
    pub urls: Vec<String>,
    pub username: Option<String>,
    pub credential: Option<String>,
}

impl std::fmt::Debug for IceServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IceServer")
            .field("urls", &self.urls)
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct WebClientService {
    inner: Arc<Inner>,
}

struct Inner {
    db: DatabaseConnection,
    settings: SettingsService,
    clock: SharedClock,
    punches: RwLock<Option<Arc<dyn WebPunches>>>,
    table: Table,
    per_user: KeyedLimit,
    per_device: KeyedLimit,
}

impl WebClientService {
    pub fn new(db: DatabaseConnection, settings: SettingsService, clock: SharedClock) -> Self {
        Self {
            inner: Arc::new(Inner {
                db,
                settings,
                per_user: KeyedLimit::new(WEB_SESSIONS_PER_USER_PER_MINUTE, Arc::clone(&clock)),
                per_device: KeyedLimit::new(WEB_SESSIONS_PER_DEVICE_PER_MINUTE, Arc::clone(&clock)),
                clock,
                punches: RwLock::default(),
                table: Table::default(),
            }),
        }
    }

    /// Without one every open is [`WebClientError::Unavailable`].
    pub fn set_punches(&self, punches: Arc<dyn WebPunches>) {
        *self
            .inner
            .punches
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(punches);
    }

    fn punches(&self) -> Option<Arc<dyn WebPunches>> {
        let punches = self.inner.punches.read();
        punches.unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// A device that does not exist and one the user may not control are the same `NotFound`.
    pub async fn open(&self, by: Opener, req: OpenRequest) -> Result<Opened, WebClientError> {
        let inner = &self.inner;
        if !inner.settings.snapshot().get(&WEB_CLIENT_ENABLED) {
            return Err(WebClientError::Disabled);
        }
        let punches = self.punches().ok_or(WebClientError::Unavailable)?;
        inner
            .per_user
            .check(&by.user_id.to_string())
            .map_err(WebClientError::RateLimited)?;
        if !self.may_control(by, &req.device_id).await? {
            return Err(WebClientError::NotFound);
        }
        inner
            .per_device
            .check(&req.device_id)
            .map_err(WebClientError::RateLimited)?;
        let reservation = inner
            .table
            .reserve(by.user_id, inner.clock.elapsed())
            .ok_or(WebClientError::TooManySessions)?;
        let mut id = [0u8; SESSION_ID_BYTES];
        fill_random(&mut id)?;
        let id = URL_SAFE_NO_PAD.encode(id);
        let punched = punches
            .punch(PunchRequest {
                device_id: req.device_id.clone(),
                offer: req.offer,
                client: req.client,
            })
            .await?;
        let opened = Opened {
            id: id.clone(),
            uuid: punched.uuid,
            relay_server: punched.relay_server,
            signed_pk: punched.signed_pk,
            answer: punched.answer,
        };
        let new = NewSession {
            device_id: req.device_id,
            sender: punched.sender,
            receiver: punched.receiver,
        };
        inner
            .table
            .insert(reservation, id, new, inner.clock.elapsed());
        Ok(opened)
    }

    async fn may_control(&self, by: Opener, device_id: &str) -> Result<bool, DbErr> {
        let repo = DeviceRepository::new(self.inner.db.clone());
        let Some(device) = repo.find_by_rustdesk_id(device_id).await? else {
            return Ok(false);
        };
        let privileged = matches!(by.role, Role::Admin | Role::Owner);
        let owned = device.owner_user_id == Some(by.user_id);
        Ok(device.status == DeviceStatus::Active && (privileged || owned))
    }

    pub fn send_candidate(
        &self,
        user_id: i64,
        id: &str,
        candidate: String,
    ) -> Result<(), WebClientError> {
        let now = self.inner.clock.elapsed();
        let session = self
            .inner
            .table
            .get(user_id, id, now)
            .ok_or(WebClientError::NotFound)?;
        session.send(candidate).map_err(WebClientError::Candidate)
    }

    pub async fn candidates(
        &self,
        user_id: i64,
        id: &str,
        after: usize,
        wait: Duration,
    ) -> Result<Candidates, WebClientError> {
        let now = self.inner.clock.elapsed();
        let session = self
            .inner
            .table
            .get(user_id, id, now)
            .ok_or(WebClientError::NotFound)?;
        session
            .poll(after, wait)
            .await
            .map_err(|_| WebClientError::Closed)
    }

    pub fn close(&self, user_id: i64, id: &str) -> Option<String> {
        let now = self.inner.clock.elapsed();
        let session = self.inner.table.remove(user_id, id, now)?;
        Some(session.device_id.clone())
    }

    /// TURN credentials go with `turn:` and `turns:` URLs only, and only when a user name is set.
    pub fn ice_servers(&self) -> Vec<IceServer> {
        let snapshot = self.inner.settings.snapshot();
        let urls = snapshot.get(&WEB_CLIENT_ICE_SERVERS);
        let username = snapshot.get(&WEB_CLIENT_TURN_USERNAME);
        let credential = snapshot.get(&WEB_CLIENT_TURN_CREDENTIAL);
        ice_urls(&urls)
            .map(|url| {
                let turn = is_turn_url(url) && !username.is_empty();
                IceServer {
                    urls: vec![url.to_owned()],
                    username: turn.then(|| username.clone()),
                    credential: turn.then(|| credential.expose().to_owned()),
                }
            })
            .collect()
    }
}
