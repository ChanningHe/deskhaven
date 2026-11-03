//! The quota on ids `--deploy` registers that the registry did not know (M2-19 review): a
//! valid deploy token must not be able to flood `devices`, which the signal server loads
//! whole at startup. Marking an id that is already registered is not charged.
//!
//! - **Per client network** ([`DEPLOY_NEW_IDS_PER_IP_PER_DAY`], keyed like the other
//!   limits: IPv4 /32, IPv6 /64, one shared key for proxy-fallback clients): the signal
//!   server's own limit on new ids per IP (300 a day), so `--deploy` adds no way around it.
//! - **Per actor** ([`DEPLOY_NEW_IDS_PER_ACTOR_PER_DAY`]): one token rolled out from many
//!   networks, generous enough for a fleet rollout.
//!
//! Fixed windows of [`WINDOW`] per key on the monotonic clock, in memory (a restart forgets
//! them, like every rate limit here).

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use crate::auth::limiter::{ClientAddr, ip_key};
use crate::clock::SharedClock;
use crate::console_audit::Actor;

/// New ids `--deploy` registers per client network per [`WINDOW`].
pub const DEPLOY_NEW_IDS_PER_IP_PER_DAY: u32 = 300;
/// New ids `--deploy` registers per actor (token) per [`WINDOW`].
pub const DEPLOY_NEW_IDS_PER_ACTOR_PER_DAY: u32 = 10_000;
/// Length of one quota window.
pub const WINDOW: Duration = Duration::from_secs(24 * 60 * 60);
/// Expired windows are dropped once the table holds more than this.
const PRUNE_ABOVE: usize = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Key {
    /// `None`: the shared proxy-fallback key.
    Network(Option<IpAddr>),
    Actor(Actor),
}

/// Start (monotonic ms) and count of a key's current window.
#[derive(Clone, Copy)]
struct Window {
    start: i64,
    count: u32,
}

pub(super) struct NewIdQuota {
    clock: SharedClock,
    per_network: u32,
    per_actor: u32,
    windows: Mutex<HashMap<Key, Window>>,
}

impl NewIdQuota {
    pub fn new(clock: SharedClock) -> Self {
        Self::with_limits(
            clock,
            DEPLOY_NEW_IDS_PER_IP_PER_DAY,
            DEPLOY_NEW_IDS_PER_ACTOR_PER_DAY,
        )
    }

    fn with_limits(clock: SharedClock, per_network: u32, per_actor: u32) -> Self {
        Self {
            clock,
            per_network,
            per_actor,
            windows: Mutex::default(),
        }
    }

    /// Charges one new id to `client`'s network and to `actor`, or returns how long until
    /// the exhausted one of the two frees up (then nothing is charged).
    pub fn try_charge(&self, client: ClientAddr, actor: Actor) -> Result<(), Duration> {
        let now = self.clock.elapsed_ms();
        let network = Key::Network((!client.is_proxy_fallback).then(|| ip_key(client.addr)));
        let keys = [
            (network, self.per_network),
            (Key::Actor(actor), self.per_actor),
        ];
        let mut windows = self.lock();
        if windows.len() > PRUNE_ABOVE {
            windows.retain(|_, w| !expired(w, now));
        }
        let fresh = Window {
            start: now,
            count: 0,
        };
        let current = keys.map(|(key, limit)| {
            let window = windows.get(&key).copied().filter(|w| !expired(w, now));
            (key, limit, window.unwrap_or(fresh))
        });
        let exhausted = current
            .iter()
            .filter(|(_, limit, window)| window.count >= *limit)
            .map(|(_, _, window)| left(window, now))
            .max();
        if let Some(wait) = exhausted {
            return Err(wait);
        }
        for (key, _, window) in current {
            let count = window.count.saturating_add(1);
            windows.insert(key, Window { count, ..window });
        }
        Ok(())
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<Key, Window>> {
        // Map operations cannot leave it inconsistent.
        self.windows.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn window_ms() -> i64 {
    i64::try_from(WINDOW.as_millis()).unwrap_or(i64::MAX)
}

fn expired(window: &Window, now: i64) -> bool {
    now.saturating_sub(window.start) >= window_ms()
}

/// Time until `window` ends, at least 1 ms.
fn left(window: &Window, now: i64) -> Duration {
    let end = window.start.saturating_add(window_ms());
    let ms = u64::try_from(end.saturating_sub(now)).unwrap_or(0).max(1);
    Duration::from_millis(ms)
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;
    use std::sync::Arc;

    use super::*;
    use crate::clock::ManualClock;

    fn client(last: u8) -> ClientAddr {
        ClientAddr {
            addr: IpAddr::V4(Ipv4Addr::new(203, 0, 113, last)),
            is_proxy_fallback: false,
        }
    }

    #[test]
    fn each_network_and_each_actor_has_its_own_window() {
        let clock = Arc::new(ManualClock::at(1_000_000));
        let quota = NewIdQuota::with_limits(clock.clone(), 2, 3);
        let (a, b) = (Actor::ApiToken(1), Actor::ApiToken(2));

        assert!(quota.try_charge(client(1), a).is_ok());
        assert!(quota.try_charge(client(1), a).is_ok());
        let network_full = quota.try_charge(client(1), a);
        assert!(quota.try_charge(client(2), a).is_ok(), "another network");
        let actor_full = quota.try_charge(client(3), a);
        assert!(quota.try_charge(client(3), b).is_ok(), "another actor");

        assert_eq!(network_full, Err(WINDOW));
        assert_eq!(actor_full, Err(WINDOW));
    }

    #[test]
    fn a_refused_charge_costs_nothing_and_windows_reopen() {
        let clock = Arc::new(ManualClock::at(1_000_000));
        let quota = NewIdQuota::with_limits(clock.clone(), 1, 2);
        let actor = Actor::ApiToken(1);
        assert!(quota.try_charge(client(1), actor).is_ok());

        clock.advance(Duration::from_secs(60));
        let refused = quota.try_charge(client(1), actor);
        assert!(
            quota.try_charge(client(2), actor).is_ok(),
            "the refusal was not charged"
        );
        clock.advance(WINDOW);
        let reopened = quota.try_charge(client(1), Actor::ApiToken(9));

        assert_eq!(refused, Err(WINDOW - Duration::from_secs(60)));
        assert!(reopened.is_ok());
    }
}
