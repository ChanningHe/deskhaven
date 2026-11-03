//! M2-19 review: `SignalHandle::live_uuid` reads a registration from the server's memory,
//! before the write-behind has stored it, so the control plane (`--deploy`) cannot hand an
//! id to another machine in that window.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use async_trait::async_trait;
use deskhaven_signal::{PeerRecord, PeerStore, PeerUpsert, SignalError, StoreError};
use hbb_common::rendezvous_proto::register_pk_response::Result::OK;
use hbb_common::tokio;
use std::{sync::Arc, time::Duration};
use support::{PeerKeys, TestServer};

const ID: &str = "live-uuid-peer";

/// A store that is slow to write and keeps nothing: what is registered lives in memory only.
struct SlowStore;

#[async_trait]
impl PeerStore for SlowStore {
    async fn load_page(&self, _: Option<&str>, _: usize) -> Result<Vec<PeerRecord>, StoreError> {
        Ok(Vec::new())
    }

    async fn get(&self, _: &str) -> Result<Option<PeerRecord>, StoreError> {
        Ok(None)
    }

    async fn upsert_pk(&self, _: &PeerUpsert) -> Result<(), StoreError> {
        tokio::time::sleep(Duration::from_millis(500)).await;
        Ok(())
    }
}

#[tokio::test]
async fn a_registration_not_stored_yet_is_live() {
    // Arrange
    let server = TestServer::start_with_store(|cfg| cfg, Arc::new(SlowStore)).await;
    let keys = PeerKeys::random();
    let mut peer = server.udp().await;
    assert_eq!(peer.register_pk(ID, &keys).await, OK);

    // Act
    let live = server.handle.live_uuid(ID).await.unwrap();
    let unknown = server.handle.live_uuid("never-registered").await.unwrap();

    // Assert
    assert_eq!(live, Some(keys.uuid.to_vec()));
    assert_eq!(unknown, None);
    let handle = server.handle.clone();
    server.stop().await;
    let stopped = handle.live_uuid(ID).await;
    assert!(matches!(stopped, Err(SignalError::Stopped)), "{stopped:?}");
}
