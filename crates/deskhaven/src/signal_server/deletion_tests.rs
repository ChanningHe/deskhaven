//! M2-12 review (M5), end to end over the device registry: deleting a device through the
//! core makes the running signal server forget it (`startup::forget_deleted_devices` ->
//! `SignalHandle::reload_peer`), so the device re-registers as a new row at once.

use std::sync::Arc;

use deskhaven_config::settings::PinSet;
use deskhaven_core::clock::SystemClock;
use deskhaven_core::crypto::{MasterKey, SecretBox};
use deskhaven_core::devices::DeviceRepository;
use deskhaven_core::settings::SettingsService;
use deskhaven_core::state::{AppState, StateConfig};
use deskhaven_core::users::PasswordHasher;
use deskhaven_db::entity::device;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_signal::{ControlPlane, PeerStore};
use tokio::time::{Instant, sleep};
use tokio_util::sync::CancellationToken;

use super::CoreControl;
use super::tests::{FAST, POLL, STEP, TARGET, Target, start_launcher_with};
use crate::signal_store::SignalPeerStore;
use crate::startup::forget_deleted_devices;

async fn state(db: &TestDb) -> AppState {
    let crypto = Arc::new(SecretBox::new(&MasterKey::from_bytes(&[7; 32])));
    let (settings, _) = SettingsService::load(db.conn.clone(), crypto.clone(), PinSet::empty())
        .await
        .unwrap();
    AppState::new(
        db.conn.clone(),
        settings,
        crypto,
        String::new(),
        PasswordHasher::default(),
        Arc::new(SystemClock),
        &StateConfig::default(),
    )
}

/// The stored row of [`TARGET`] once the write-behind has stored it (within [`STEP`]).
async fn stored(repo: &DeviceRepository, not: Option<i64>) -> device::Model {
    let deadline = Instant::now() + STEP;
    loop {
        let row = repo.find_by_rustdesk_id(TARGET).await.unwrap();
        if let Some(row) = row.filter(|r| Some(r.id) != not) {
            return row;
        }
        assert!(Instant::now() < deadline, "registration never stored");
        sleep(POLL).await;
    }
}

#[tokio::test]
async fn a_deleted_device_is_forgotten_by_the_signal_server_and_registers_again() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let state = state(db).await;
        let repo = DeviceRepository::new(db.conn.clone());
        let store: Arc<dyn PeerStore> = Arc::new(SignalPeerStore::new(repo.clone()));
        let control: Arc<dyn ControlPlane> =
            Arc::new(CoreControl::new(state.devices.presence().clone()));
        let token = CancellationToken::new();
        let settings = state.settings.subscribe();
        let (running, port, _) = start_launcher_with(settings, store, control, FAST, &token).await;
        forget_deleted_devices(&state, running.status.clone());
        let mut target = Target::register(port).await;
        let first = stored(&repo, None).await;
        assert!(!target.register_peer().await, "registered");

        // Act
        assert!(state.devices.delete(first.id).await.unwrap());

        // Assert: the server asks for the key again, and the re-registration is a new row.
        let deadline = Instant::now() + STEP;
        while !target.register_peer().await {
            assert!(
                Instant::now() < deadline,
                "{}: still registered",
                db.backend
            );
            sleep(POLL).await;
        }
        Target::register(port).await;
        let again = stored(&repo, Some(first.id)).await;
        assert_ne!(again.id, first.id, "{}", db.backend);
        token.cancel();
        running.task.await.unwrap();
    })
    .await;
}
