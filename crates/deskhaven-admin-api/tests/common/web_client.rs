// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]
// Each test crate uses a subset.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use axum::http::Method;
use deskhaven_admin_api::AdminRoutes;
use deskhaven_config::bootstrap::{LoadOptions, load};
use deskhaven_config::settings::PinSet;
use deskhaven_core::clock::SharedClock;
use deskhaven_core::users::Role;
use deskhaven_core::web_client::{
    CandidateRejected, CandidateSink, CandidateSource, NextCandidate, PunchError, PunchFuture,
    PunchRequest, Punched, WebPunches,
};
use deskhaven_db::testing::TestDb;
use serde_json::json;
use tokio::sync::{mpsc, oneshot};

use super::devices::register;
use super::{Console, Harness, Reply, harness_behind_proxy, harness_on_clock, harness_pinned};

pub const SESSIONS: &str = "/admin/api/v1/web-client/sessions";
pub const OFFER: &str = "webrtc://eyJ0eXBlIjoib2ZmZXIifQ==";
pub const ANSWER: &str = "webrtc://eyJ0eXBlIjoiYW5zd2VyIn0=";
pub const UUID: &str = "e4c181d0-41c7-4ae6-8ef0-36581b6b2b12";
pub const RELAY: &str = "relay.example.com:21117";
pub const SIGNED_PK: [u8; 4] = [1, 2, 3, 4];

pub struct Route {
    pub request: PunchRequest,
    pub to_device: Arc<Mutex<Vec<String>>>,
    /// Drop it to close the route.
    pub from_device: mpsc::UnboundedSender<String>,
}

#[derive(Default)]
pub struct FakePunches {
    fail: Mutex<Option<PunchError>>,
    reject: Mutex<Option<CandidateRejected>>,
    routes: Mutex<Vec<Route>>,
    park: Mutex<Option<oneshot::Receiver<()>>>,
}

impl FakePunches {
    pub fn install(h: &Harness) -> Arc<Self> {
        let fake = Arc::new(Self::default());
        h.state.web_client.set_punches(fake.clone());
        fake
    }

    pub fn fail_with(&self, err: Option<PunchError>) {
        *self.fail.lock().unwrap() = err;
    }

    pub fn reject_candidates(&self, err: CandidateRejected) {
        *self.reject.lock().unwrap() = Some(err);
    }

    /// Dropping a route's `from_device` closes it.
    pub fn take(&self) -> Vec<Route> {
        std::mem::take(&mut *self.routes.lock().unwrap())
    }

    /// The next punch answers only once the returned sender fires (or is dropped).
    pub fn park_next(&self) -> oneshot::Sender<()> {
        let (release, parked) = oneshot::channel();
        *self.park.lock().unwrap() = Some(parked);
        release
    }

    pub fn punches(&self) -> usize {
        self.routes.lock().unwrap().len()
    }
}

struct Sink {
    sent: Arc<Mutex<Vec<String>>>,
    reject: Option<CandidateRejected>,
}

impl CandidateSink for Sink {
    fn send(&self, candidate: String) -> Result<(), CandidateRejected> {
        if let Some(err) = self.reject {
            return Err(err);
        }
        self.sent.lock().unwrap().push(candidate);
        Ok(())
    }
}

struct Source(mpsc::UnboundedReceiver<String>);

impl CandidateSource for Source {
    fn next(&mut self) -> NextCandidate<'_> {
        Box::pin(self.0.recv())
    }
}

impl WebPunches for FakePunches {
    fn punch(&self, req: PunchRequest) -> PunchFuture<'_> {
        let fail = self.fail.lock().unwrap().clone();
        let reject = *self.reject.lock().unwrap();
        let result = match fail {
            Some(err) => Err(err),
            None => {
                let to_device = Arc::new(Mutex::new(Vec::new()));
                let (from_device, rx) = mpsc::unbounded_channel();
                let punched = Punched {
                    uuid: UUID.to_owned(),
                    relay_server: RELAY.to_owned(),
                    signed_pk: SIGNED_PK.to_vec(),
                    answer: req.offer.as_ref().map(|_| ANSWER.to_owned()),
                    sender: Box::new(Sink {
                        sent: to_device.clone(),
                        reject,
                    }),
                    receiver: Box::new(Source(rx)),
                };
                self.routes.lock().unwrap().push(Route {
                    request: req,
                    to_device,
                    from_device,
                });
                Ok(punched)
            }
        };
        let parked = self.park.lock().unwrap().take();
        Box::pin(async move {
            if let Some(release) = parked {
                let _ = release.await;
            }
            result
        })
    }
}

/// Pinned on by a real configuration file: the only way to turn it on.
pub fn enabled() -> PinSet {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deskhaven.toml");
    std::fs::write(&path, "[settings]\n\"web_client.enabled\" = true\n").unwrap();
    let options = LoadOptions {
        config_path: Some(path),
        search_paths: Vec::new(),
        cli: Default::default(),
    };
    let loaded = load(&options).unwrap();
    PinSet::from_bootstrap(&loaded.config.settings).unwrap()
}

pub async fn enabled_harness(db: &TestDb) -> Harness {
    harness_pinned(db, AdminRoutes::new(), enabled()).await
}

pub async fn enabled_harness_behind_proxy(db: &TestDb) -> Harness {
    harness_behind_proxy(db, enabled()).await
}

pub async fn enabled_harness_on_clock(db: &TestDb, clock: SharedClock) -> Harness {
    harness_on_clock(db, enabled(), clock).await
}

pub async fn owned_device(h: &Harness, admin: &Console, rustdesk_id: &str, owner: i64) {
    let id = register(h, rustdesk_id).await;
    let path = format!("/admin/api/v1/devices/{id}");
    let done = h
        .write(
            Method::PATCH,
            &path,
            admin,
            json!({ "owner_user_id": owner }),
        )
        .await;
    assert_eq!(done.status, 200, "{:?}", done.body);
}

/// `alice`, a plain user, owns device `111111111`; the owner owns `222222222`.
pub struct Setup {
    pub fake: Arc<FakePunches>,
    pub owner: Console,
    pub alice: Console,
}

pub const ALICE_DEVICE: &str = "111111111";
pub const OWNER_DEVICE: &str = "222222222";

pub async fn setup(h: &Harness) -> Setup {
    let fake = FakePunches::install(h);
    let owner = h.console("owner", Role::Owner).await;
    let alice = h.console("alice", Role::User).await;
    owned_device(h, &owner, ALICE_DEVICE, alice.user_id).await;
    owned_device(h, &owner, OWNER_DEVICE, owner.user_id).await;
    Setup { fake, owner, alice }
}

pub async fn open(h: &Harness, console: &Console, device_id: &str, offer: bool) -> Reply {
    let mut body = json!({ "device_id": device_id });
    if offer {
        body["offer"] = json!(OFFER);
    }
    h.write(Method::POST, SESSIONS, console, body).await
}

pub async fn opened(h: &Harness, console: &Console, device_id: &str) -> String {
    let reply = open(h, console, device_id, true).await;
    assert_eq!(reply.status, 201, "{:?}", reply.body);
    reply.body["session_id"].as_str().unwrap().to_owned()
}

pub fn candidates_path(id: &str) -> String {
    format!("{SESSIONS}/{id}/candidates")
}
