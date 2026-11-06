//! The readiness probe's database ping, shared for [`PING_TTL`] (M4-05). `/readyz` is public
//! and probe logs are at debug, so without this every request (a flood included) took a pool
//! connection for a ping. One ping now answers every probe within the TTL, and concurrent
//! probes after it expired wait for a single ping instead of starting one each.

use std::sync::Arc;
use std::time::{Duration, Instant};

use sea_orm::DatabaseConnection;
use tokio::sync::Mutex;

use crate::log_safe;

/// How long one ping's result answers probes. Orchestrators probe every few seconds, so a
/// failure still shows within a second.
pub const PING_TTL: Duration = Duration::from_secs(1);

/// A ping result: `Err` carries the not-ready reason.
type Outcome = Result<(), String>;

#[derive(Clone)]
pub struct CachedPing {
    db: DatabaseConnection,
    ttl: Duration,
    timeout: Duration,
    last: Arc<Mutex<Option<(Instant, Outcome)>>>,
}

impl CachedPing {
    /// Pings `db` at most once per `ttl`; a ping slower than `timeout` fails.
    pub fn new(db: DatabaseConnection, ttl: Duration, timeout: Duration) -> Self {
        Self {
            db,
            ttl,
            timeout,
            last: Arc::default(),
        }
    }

    pub async fn ping(&self) -> Outcome {
        let mut last = self.last.lock().await;
        if let Some((at, outcome)) = last.as_ref()
            && at.elapsed() < self.ttl
        {
            return outcome.clone();
        }
        let outcome = match tokio::time::timeout(self.timeout, self.db.ping()).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(format!(
                "database ping failed: {}",
                log_safe::render_error(&err)
            )),
            Err(_) => Err("database ping timed out".to_owned()),
        };
        *last = Some((Instant::now(), outcome.clone()));
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIMEOUT: Duration = Duration::from_secs(2);

    async fn memory_db() -> DatabaseConnection {
        sea_orm::Database::connect("sqlite::memory:").await.unwrap()
    }

    #[tokio::test]
    async fn a_result_answers_probes_until_it_expires() {
        // Arrange
        let db = memory_db().await;
        let ttl = Duration::from_millis(200);
        let ping = CachedPing::new(db.clone(), ttl, TIMEOUT);
        assert_eq!(ping.ping().await, Ok(()));

        // Act: the database goes away right after a successful ping.
        db.close().await.unwrap();

        // Assert
        assert_eq!(ping.ping().await, Ok(()), "cached");
        tokio::time::sleep(ttl * 2).await;
        let expired = ping.ping().await;
        assert!(expired.is_err(), "{expired:?}");
    }
}
