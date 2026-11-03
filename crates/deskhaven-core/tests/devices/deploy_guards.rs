//! `--deploy` guards (M2-19 review): the quota on new ids per source network, the signal
//! server's live registrations, and the registry-changed hook that loads new ids into its
//! memory.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use deskhaven_core::devices::{
    DEPLOY_NEW_IDS_PER_IP_PER_DAY, DeployOutcome, DeployRequest, LiveLookup, LiveRegistrations,
    LiveUnknown,
};
use deskhaven_db::testing::{TestDb, each_migrated_backend};

use super::deploy::{PK, request, requester};
use super::support::{ID, ctx, uuid_of};

const OTHER_ID: &str = "246813579";

/// The signal server's memory, as the test sets it; `None`: it does not answer in time.
struct Live {
    known: Option<Vec<(String, Vec<u8>)>>,
    asked: AtomicUsize,
}

impl Live {
    fn new(known: Option<Vec<(String, Vec<u8>)>>) -> Arc<Self> {
        Arc::new(Self {
            known,
            asked: AtomicUsize::new(0),
        })
    }
}

impl LiveRegistrations for Live {
    fn uuid_of<'a>(&'a self, rustdesk_id: &'a str) -> LiveLookup<'a> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        let found = self.known.as_ref().map(|known| {
            let entry = known.iter().find(|(id, _)| id == rustdesk_id);
            entry.map(|(_, uuid)| uuid.clone())
        });
        Box::pin(async move { found.ok_or(LiveUnknown) })
    }
}

fn from(ip: IpAddr, rustdesk_id: &str) -> DeployRequest {
    DeployRequest {
        by: requester(ip),
        ..request(rustdesk_id, &uuid_of(rustdesk_id), &PK)
    }
}

#[tokio::test]
async fn new_ids_are_limited_per_source_network_but_marking_is_not() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.device(ID).await;
        c.require_deploy().await;
        let ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 50));
        let other = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 51));
        for n in 0..DEPLOY_NEW_IDS_PER_IP_PER_DAY {
            let outcome = c.deploy(from(ip, &format!("new-{n:06}"))).await;
            assert_eq!(outcome, DeployOutcome::Deployed, "{}: {n}", db.backend);
        }

        // Act
        let over = c.deploy(from(ip, "one-too-many")).await;
        let marking = c.deploy(from(ip, ID)).await;
        let elsewhere = c.deploy(from(other, "one-too-many")).await;

        // Assert
        let b = db.backend;
        assert!(
            matches!(over, DeployOutcome::TooMany { .. }),
            "{b}: {over:?}"
        );
        assert_eq!(
            marking,
            DeployOutcome::Deployed,
            "{b}: no new id, not charged"
        );
        assert_eq!(elsewhere, DeployOutcome::Deployed, "{b}: per network");
    })
    .await;
}

/// A device the signal server registered a moment ago may not be stored yet (write-behind):
/// its live uuid wins, so a deploy cannot take the id from it.
#[tokio::test]
async fn an_id_live_in_the_signal_server_with_another_uuid_is_taken() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        c.require_deploy().await;
        let live = vec![
            (ID.to_owned(), b"live-machine".to_vec()),
            (OTHER_ID.to_owned(), uuid_of(OTHER_ID)),
        ];
        c.devices.set_live_registrations(Live::new(Some(live)));

        let taken = c.deploy(request(ID, &uuid_of(ID), &PK)).await;
        let same = c.deploy(request(OTHER_ID, &uuid_of(OTHER_ID), &PK)).await;

        let b = db.backend;
        assert_eq!(taken, DeployOutcome::IdTaken, "{b}");
        let repo = deskhaven_core::devices::DeviceRepository::new(c.conn.clone());
        assert!(repo.find_by_rustdesk_id(ID).await.unwrap().is_none(), "{b}");
        assert_eq!(
            same,
            DeployOutcome::Deployed,
            "{b}: its own live registration"
        );
    })
    .await;
}

/// A signal server that cannot tell in time is no green light: the deploy is refused as
/// retryable and nothing is written.
#[tokio::test]
async fn an_unanswered_live_lookup_is_retryable_and_writes_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        c.require_deploy().await;
        c.devices.set_live_registrations(Live::new(None));

        let outcome = c.deploy(request(ID, &uuid_of(ID), &PK)).await;

        let b = db.backend;
        assert!(
            matches!(outcome, DeployOutcome::Unavailable { .. }),
            "{b}: {outcome:?}"
        );
        let repo = deskhaven_core::devices::DeviceRepository::new(c.conn.clone());
        assert!(repo.find_by_rustdesk_id(ID).await.unwrap().is_none(), "{b}");
        assert!(c.audit_rows().await.is_empty(), "{b}");
    })
    .await;
}

/// Only a new id makes the signal server re-read it: marking an id it already holds must not
/// replace its in-memory keys with the stored ones.
#[tokio::test]
async fn only_a_new_id_runs_the_registry_changed_hooks() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        c.device(ID).await;
        c.require_deploy().await;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        c.devices.on_registry_changed(Arc::new(move |id: &str| {
            sink.lock().unwrap().push(id.to_owned());
        }));

        c.deploy(request(ID, &uuid_of(ID), &PK)).await;
        c.deploy(request(OTHER_ID, &uuid_of(OTHER_ID), &PK)).await;
        c.deploy(request(ID, b"another-machine", &PK)).await;

        let seen = seen.lock().unwrap().clone();
        assert_eq!(
            seen,
            [OTHER_ID],
            "{}: not for marking or ID_TAKEN",
            db.backend
        );
    })
    .await;
}

/// Deploying an id that is already deployed answers OK at once: no lookup, no write.
#[tokio::test]
async fn a_repeated_deploy_neither_asks_the_signal_server_nor_writes() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        c.require_deploy().await;
        c.deploy(request(ID, &uuid_of(ID), &PK)).await;
        let live = Live::new(Some(Vec::new()));
        c.devices.set_live_registrations(live.clone());
        let audited = c.audit_rows().await.len();

        let again = c.deploy(request(ID, &uuid_of(ID), &PK)).await;

        let b = db.backend;
        assert_eq!(again, DeployOutcome::Deployed, "{b}");
        assert_eq!(live.asked.load(Ordering::SeqCst), 0, "{b}");
        assert_eq!(c.audit_rows().await.len(), audited, "{b}");
    })
    .await;
}
