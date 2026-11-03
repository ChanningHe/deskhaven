use std::sync::Arc;

use deskhaven_db::testing::{TestDb, each_migrated_backend};
use tokio::task::JoinSet;

use super::*;

/// Concurrent heartbeats of one device (a clone, or requests racing) per round.
const RACERS: usize = 8;
const ROUNDS: usize = 5;

fn effective(modified_at: i64, keys: &[&str]) -> EffectiveStrategy {
    EffectiveStrategy {
        options: keys
            .iter()
            .map(|k| ((*k).to_owned(), "v".to_owned()))
            .collect(),
        modified_at,
        applied: Vec::new(),
    }
}

async fn stored(db: &TestDb, rustdesk_id: &str) -> Keys {
    let row = StoredKeys::find_by_id(rustdesk_id.to_owned())
        .one(&db.conn)
        .await
        .unwrap();
    row.map(|r| decode(&r.option_keys)).unwrap_or_default()
}

/// Pushes with different keys race each other and a confirmation that would shrink the
/// set: whatever succeeded is still in the stored set, so no key a device may hold is lost.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn racing_pushes_and_a_confirmation_never_lose_a_sent_key() {
    each_migrated_backend(async |db: &TestDb| {
        let b = db.backend;
        for round in 0..ROUNDS {
            // Arrange: {a, b} sent, then {a}: confirming the latter shrinks to {a}.
            let id = format!("10000000{round}");
            let deliveries = Arc::new(StrategyDeliveries::new(db.conn.clone()));
            deliveries
                .prepare(&id, &effective(1, &["a", "b"]))
                .await
                .unwrap();
            deliveries
                .prepare(&id, &effective(2, &["a"]))
                .await
                .unwrap();

            // Act
            let mut racers = JoinSet::new();
            for n in 0..RACERS {
                let (deliveries, id) = (Arc::clone(&deliveries), id.clone());
                racers.spawn(async move {
                    let key = format!("k{n}");
                    let version = 10 + i64::try_from(n).unwrap();
                    let sent = deliveries.prepare(&id, &effective(version, &[&key])).await;
                    sent.ok().map(|_| key)
                });
            }
            let confirmer = Arc::clone(&deliveries);
            let confirm_id = id.clone();
            racers.spawn(async move {
                confirmer.confirm(&confirm_id, 2).await.unwrap();
                None
            });
            let sent: Vec<String> = racers.join_all().await.into_iter().flatten().collect();

            // Assert
            let kept = stored(db, &id).await;
            assert!(!sent.is_empty(), "{b}: every push failed");
            for key in &sent {
                assert!(kept.contains(key), "{b}: {key} lost from {kept:?}");
            }
        }
    })
    .await;
}

/// Beyond the capacity the pending confirmations are dropped: a later confirmation then
/// changes nothing, and the stored set stays a superset.
#[tokio::test]
async fn a_full_pending_table_is_cleared_and_the_stored_set_kept() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let deliveries = StrategyDeliveries::with_capacity(db.conn.clone(), 2);
        deliveries
            .prepare("1", &effective(1, &["a", "b"]))
            .await
            .unwrap();
        deliveries
            .prepare("1", &effective(2, &["a"]))
            .await
            .unwrap();
        deliveries.prepare("2", &effective(2, &[])).await.unwrap();

        // Act
        deliveries.prepare("3", &effective(2, &[])).await.unwrap();
        let after_clear = deliveries.pending_len();
        deliveries.confirm("1", 2).await.unwrap();

        // Assert
        let b = db.backend;
        assert_eq!(after_clear, 1, "{b}: cleared, then the new one");
        let kept: Vec<String> = stored(db, "1").await.into_iter().collect();
        assert_eq!(kept, ["a", "b"], "{b}: no confirmation left to shrink it");
    })
    .await;
}
