//! M2-19 review, end to end: `--deploy` through the core and the running signal server
//! (`startup::share_registry_with_deploy`). A deployed new id is loaded into the server's
//! memory at once (registry-changed hook -> `reload_peer`), and an id the server holds for
//! another uuid is refused.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use base64_022::Engine as _;
use base64_022::engine::general_purpose::STANDARD;
use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::console_audit::Actor;
use deskhaven_core::devices::{DeployOutcome, DeployRequest, DeviceRepository, Requester};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_signal::{ControlPlane, PeerStore};
use tokio::time::{Instant, sleep};
use tokio_util::sync::CancellationToken;

use super::CoreControl;
use super::deletion_tests::state;
use super::tests::{FAST, POLL, STEP, TARGET, Target, start_launcher_with};
use crate::signal_store::SignalPeerStore;
use crate::startup::share_registry_with_deploy;

const NEW_ID: &str = "555555555";

fn deploy(id: &str, uuid: &[u8]) -> DeployRequest {
    DeployRequest {
        rustdesk_id: id.to_owned(),
        uuid: STANDARD.encode(uuid),
        pk: STANDARD.encode([5u8; 32]),
        by: Requester {
            actor: Actor::ApiToken(1),
            client: ClientAddr {
                addr: IpAddr::V4(Ipv4Addr::new(198, 51, 100, 9)),
                is_proxy_fallback: false,
            },
        },
    }
}

#[tokio::test]
async fn deploy_follows_and_feeds_the_signal_servers_registry() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let state = state(db).await;
        state
            .settings
            .update(deskhaven_core::settings::UpdateRequest {
                changes: vec![deskhaven_config::settings::Change {
                    key: "devices.require_deploy".to_owned(),
                    value: serde_json::json!(true),
                    version: state
                        .settings
                        .snapshot()
                        .view("devices.require_deploy")
                        .unwrap()
                        .version,
                }],
                actor: None,
                reason: None,
            })
            .await
            .unwrap();
        let repo = DeviceRepository::new(db.conn.clone());
        let store: Arc<dyn PeerStore> = Arc::new(SignalPeerStore::new(repo));
        let control: Arc<dyn ControlPlane> =
            Arc::new(CoreControl::new(state.devices.presence().clone()));
        let token = CancellationToken::new();
        let settings = state.settings.subscribe();
        let (running, port, _) = start_launcher_with(settings, store, control, FAST, &token).await;
        share_registry_with_deploy(&state, running.status.clone());
        Target::register(port).await;

        // Act
        let taken = state
            .devices
            .deploy(deploy(TARGET, b"another-machine"))
            .await;
        let own = state.devices.deploy(deploy(TARGET, b"target-uuid")).await;
        let new = state.devices.deploy(deploy(NEW_ID, b"new-machine")).await;

        // Assert
        let b = db.backend;
        assert_eq!(taken.unwrap(), DeployOutcome::IdTaken, "{b}");
        assert_eq!(own.unwrap(), DeployOutcome::Deployed, "{b}");
        assert_eq!(new.unwrap(), DeployOutcome::Deployed, "{b}");
        let deadline = Instant::now() + STEP;
        while running
            .status
            .live_uuid(NEW_ID)
            .await
            .ok()
            .flatten()
            .as_deref()
            != Some(b"new-machine")
        {
            assert!(Instant::now() < deadline, "{b}: never loaded into memory");
            sleep(POLL).await;
        }
        token.cancel();
        running.task.await.unwrap();
    })
    .await;
}
