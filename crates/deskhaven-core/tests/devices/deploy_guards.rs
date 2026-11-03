//! `--deploy` guards (M2-19 review): the quota on new ids per source network, the signal
//! server's live registrations, and the registry-changed hook that keeps its memory in step.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::future::Future;
use std::net::{IpAddr, Ipv4Addr};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use deskhaven_core::devices::{
    DEPLOY_NEW_IDS_PER_IP_PER_DAY, DeployOutcome, DeployRequest, LiveRegistrations,
};
use deskhaven_db::testing::{TestDb, each_migrated_backend};

use super::deploy::{PK, request, requester};
use super::support::{ID, ctx, uuid_of};

/// The signal server's memory, as the test sets it.
struct Live(Mutex<Vec<(String, Vec<u8>)>>);

impl LiveRegistrations for Live {
    fn uuid_of<'a>(
        &'a self,
        rustdesk_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Option<Vec<u8>>> + Send + 'a>> {
        let found = self
            .0
            .lock()
            .unwrap()
            .iter()
            .find(|(id, _)| id == rustdesk_id)
            .map(|(_, uuid)| uuid.clone());
        Box::pin(async move { found })
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
            ("246813579".to_owned(), uuid_of("246813579")),
        ];
        c.devices
            .set_live_registrations(Arc::new(Live(Mutex::new(live))));

        let taken = c.deploy(request(ID, &uuid_of(ID), &PK)).await;
        let same = c
            .deploy(request("246813579", &uuid_of("246813579"), &PK))
            .await;

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

/// Every deploy that wrote tells the signal server to re-read the id (as deletion does).
#[tokio::test]
async fn a_deploy_that_writes_runs_the_registry_changed_hooks() {
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
        c.deploy(request("246813579", &uuid_of("246813579"), &PK))
            .await;
        c.deploy(request(ID, b"another-machine", &PK)).await;

        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen, [ID, "246813579"], "{}: not for ID_TAKEN", db.backend);
    })
    .await;
}
