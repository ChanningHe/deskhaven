//! In-memory stand-in for upstream `database.rs` (sqlx 0.6 + deadpool, not vendored).
//!
//! Keeps the query signatures `peer.rs` calls. Nothing survives a restart. M1-05 replaces this
//! module with the persistent `PeerStore` trait.

// SECURITY: because uuid/pk are lost on restart, the first device to register an id after a
// restart owns it: the UUID_MISMATCH check (anti id-hijack) only covers ids registered since
// the last start. Do not wire this stub into `deskhaven serve`; M1-05 must replace it first.

use hbb_common::{anyhow::anyhow, tokio::sync::Mutex, ResultType};
use std::{collections::HashMap, sync::Arc};

/// Upstream `peer` row, reduced to the columns `peer.rs` reads.
#[derive(Clone, Default)]
pub struct Peer {
    pub guid: Vec<u8>,
    pub uuid: Vec<u8>,
    pub pk: Vec<u8>,
    pub info: String,
}

/// Rows keyed by peer id (upstream: `unique index index_peer_id on peer (id)`).
#[derive(Clone, Default)]
pub struct Database {
    peers: Arc<Mutex<HashMap<String, Peer>>>,
}

impl Database {
    pub async fn get_peer(&self, id: &str) -> ResultType<Option<Peer>> {
        Ok(self.peers.lock().await.get(id).cloned())
    }

    /// Fails if `id` already exists, like upstream's unique index on `id`.
    pub async fn insert_peer(
        &self,
        id: &str,
        uuid: &[u8],
        pk: &[u8],
        info: &str,
    ) -> ResultType<Vec<u8>> {
        let mut peers = self.peers.lock().await;
        if peers.contains_key(id) {
            return Err(anyhow!("peer id already exists: {id}"));
        }
        let guid = uuid::Uuid::new_v4().as_bytes().to_vec();
        let peer = Peer {
            guid: guid.clone(),
            uuid: uuid.to_vec(),
            pk: pk.to_vec(),
            info: info.to_owned(),
        };
        peers.insert(id.to_owned(), peer);
        Ok(guid)
    }

    /// `update peer set id=?, pk=?, info=? where guid=?`; like SQL, an unknown guid is a no-op.
    #[allow(clippy::ptr_arg)] // signature kept identical to upstream
    pub async fn update_pk(
        &self,
        guid: &Vec<u8>,
        id: &str,
        pk: &[u8],
        info: &str,
    ) -> ResultType<()> {
        let mut peers = self.peers.lock().await;
        let Some(old_id) = peers
            .iter()
            .find(|(_, peer)| &peer.guid == guid)
            .map(|(old_id, _)| old_id.clone())
        else {
            return Ok(());
        };
        if old_id != id && peers.contains_key(id) {
            return Err(anyhow!("peer id already exists: {id}"));
        }
        if let Some(old) = peers.remove(&old_id) {
            let peer = Peer {
                pk: pk.to_vec(),
                info: info.to_owned(),
                ..old
            };
            peers.insert(id.to_owned(), peer);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Database;
    use hbb_common::tokio;

    #[tokio::test]
    async fn insert_get_update_round_trip() {
        let db = Database::default();
        assert!(db.get_peer("a").await.unwrap().is_none());

        let guid = db.insert_peer("a", b"uuid", b"pk1", "{}").await.unwrap();
        assert!(db.insert_peer("a", b"uuid", b"pk1", "{}").await.is_err());

        db.update_pk(&guid, "b", b"pk2", "{\"ip\":\"x\"}")
            .await
            .unwrap();
        assert!(db.get_peer("a").await.unwrap().is_none());
        let peer = db.get_peer("b").await.unwrap().unwrap();
        assert_eq!(peer.guid, guid);
        assert_eq!(peer.uuid, b"uuid");
        assert_eq!(peer.pk, b"pk2");
        assert_eq!(peer.info, "{\"ip\":\"x\"}");
    }

    #[tokio::test]
    async fn update_rejects_taken_id_and_ignores_unknown_guid() {
        let db = Database::default();
        let guid_a = db.insert_peer("a", b"", b"", "").await.unwrap();
        db.insert_peer("b", b"", b"", "").await.unwrap();

        assert!(db.update_pk(&guid_a, "b", b"", "").await.is_err());
        db.update_pk(&b"unknown".to_vec(), "c", b"", "")
            .await
            .unwrap();
        assert!(db.get_peer("c").await.unwrap().is_none());
    }
}
