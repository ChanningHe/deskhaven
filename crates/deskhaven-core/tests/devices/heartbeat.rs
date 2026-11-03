//! The heartbeat (M2-14, 02 §4.1): identity, presence, sysinfo request, disconnects and the
//! strategy with its removed-key deletes; the steady path touches no database.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use deskhaven_core::clock::{Clock, ManualClock};
use deskhaven_core::devices::{
    HEARTBEAT_MIN_INTERVAL, Heartbeat, HeartbeatOutcome, HeartbeatReply, Presets,
};
use deskhaven_core::strategies::{NewStrategy, Options, StrategyUpdate, Target};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::DatabaseConnection;

use super::support::{Ctx, ID, T0_MS, b64, ctx, ctx_on, report, sysinfo, uuid_of};

const IP: IpAddr = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 4));
/// The client's idle period is 15 s; 3 s with live connections.
const TICK: Duration = Duration::from_secs(3);

fn opts(pairs: &[(&str, &str)]) -> Options {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn beat(conns: &[i32], modified_at: i64) -> Heartbeat {
    Heartbeat {
        rustdesk_id: ID.to_owned(),
        uuid: b64(&uuid_of(ID)),
        ip: IP,
        conns: conns.to_vec(),
        modified_at,
    }
}

/// One tick later, a heartbeat of [`ID`] that must be answered.
async fn send(c: &Ctx, conns: &[i32], modified_at: i64) -> HeartbeatReply {
    c.clock.advance(TICK);
    match c.devices.heartbeat(beat(conns, modified_at)).await.unwrap() {
        HeartbeatOutcome::Reply(reply) => reply,
        other => panic!("expected a reply, got {other:?}"),
    }
}

/// The client: applies a strategy and keeps the version it was sent (`sync.rs:261-275`).
struct Client {
    modified_at: i64,
}

impl Client {
    async fn beat(&mut self, c: &Ctx) -> HeartbeatReply {
        let reply = send(c, &[], self.modified_at).await;
        if let Some(push) = &reply.strategy {
            self.modified_at = push.modified_at;
        }
        reply
    }
}

/// `strategy.config_options` of `reply` (empty when none was sent).
fn options(reply: &HeartbeatReply) -> Options {
    reply
        .strategy
        .as_ref()
        .map(|s| s.config_options.clone())
        .unwrap_or_default()
}

impl Ctx {
    async fn global(&self, pairs: &[(&str, &str)]) -> i64 {
        let created = self
            .strategies
            .create(NewStrategy::new("global", opts(pairs)));
        let strategy = created.await.unwrap();
        self.strategies
            .assign(Target::Global, strategy.id)
            .await
            .unwrap();
        strategy.id
    }

    async fn set_options(&self, strategy_id: i64, pairs: &[(&str, &str)]) {
        let update = StrategyUpdate {
            options: Some(opts(pairs)),
            ..StrategyUpdate::default()
        };
        self.strategies.update(strategy_id, update).await.unwrap();
    }
}

/// A clone of `conn` that records the SQL of every statement it runs.
fn counted(conn: &DatabaseConnection) -> (DatabaseConnection, Arc<Mutex<Vec<String>>>) {
    let mut conn = conn.clone();
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&log);
    conn.set_metric_callback(move |info| sink.lock().unwrap().push(info.statement.sql.clone()));
    (conn, log)
}

fn take(log: &Mutex<Vec<String>>) -> Vec<String> {
    std::mem::take(&mut *log.lock().unwrap())
}

#[tokio::test]
async fn an_unknown_id_and_a_wrong_uuid_are_not_found_alike_and_change_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.device(ID).await;
        c.devices.disconnects().disconnect(ID, &[12]).unwrap();
        let unknown = Heartbeat {
            rustdesk_id: "555555555".to_owned(),
            ..beat(&[12], 0)
        };
        let wrong = Heartbeat {
            uuid: b64(b"another-uuid"),
            ..beat(&[12], 0)
        };

        // Act
        let unknown = c.devices.heartbeat(unknown).await.unwrap();
        let wrong = c.devices.heartbeat(wrong).await.unwrap();

        // Assert
        let b = db.backend;
        assert_eq!(unknown, HeartbeatOutcome::NotFound, "{b}");
        assert_eq!(wrong, HeartbeatOutcome::NotFound, "{b}");
        assert!(!c.devices.presence().is_online(ID), "{b}");
        assert!(
            c.devices.disconnects().is_pending(ID),
            "{b}: not handed out"
        );
    })
    .await;
}

#[tokio::test]
async fn a_device_is_asked_for_its_sysinfo_until_it_reports_one() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.device(ID).await;
        c.device("987654321").await;
        c.report(report("987654321", sysinfo("other"), Presets::default()))
            .await;

        // Act
        let before = send(&c, &[], 0).await;
        let again = send(&c, &[], 0).await;
        c.report(report(ID, sysinfo("pc"), Presets::default()))
            .await;
        let after = send(&c, &[], 0).await;
        let other = Heartbeat {
            rustdesk_id: "987654321".to_owned(),
            uuid: b64(&uuid_of("987654321")),
            ..beat(&[], 0)
        };
        let other = c.devices.heartbeat(other).await.unwrap();

        // Assert
        let b = db.backend;
        assert!(before.sysinfo && again.sysinfo, "{b}");
        assert!(!after.sysinfo, "{b}: stored, no longer asked");
        let HeartbeatOutcome::Reply(other) = other else {
            panic!("{b}: {other:?}");
        };
        assert!(!other.sysinfo, "{b}: read with the identity");
    })
    .await;
}

#[tokio::test]
async fn a_heartbeat_updates_presence_and_one_too_soon_is_refused() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.device(ID).await;
        send(&c, &[5], 0).await;
        c.devices.disconnects().disconnect(ID, &[7]).unwrap();

        // Act
        c.clock
            .advance(HEARTBEAT_MIN_INTERVAL - Duration::from_millis(1));
        let early = c.devices.heartbeat(beat(&[7], 0)).await.unwrap();
        let state = c.devices.presence().get(ID).unwrap();

        // Assert
        let b = db.backend;
        assert_eq!(early, HeartbeatOutcome::TooFrequent, "{b}");
        assert!(state.online, "{b}");
        assert_eq!(state.conns, vec![5], "{b}: the refused one changed nothing");
        assert_eq!(state.ip, Some(IP), "{b}");
        assert_eq!(state.last_seen_ms, Some(T0_MS + 3_000), "{b}");
        let reply = send(&c, &[7], 0).await;
        assert_eq!(reply.disconnect, vec![7], "{b}: still queued");
    })
    .await;
}

#[tokio::test]
async fn disconnects_are_handed_out_once_and_only_for_live_connections() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.device(ID).await;
        c.devices.disconnects().disconnect(ID, &[12, 13]).unwrap();

        // Act
        let first = send(&c, &[11, 12], 0).await;
        let second = send(&c, &[11, 12], 0).await;

        // Assert
        let b = db.backend;
        assert_eq!(
            first.disconnect,
            vec![12],
            "{b}: 13 is gone, 11 not asked for"
        );
        assert!(second.disconnect.is_empty(), "{b}");
    })
    .await;
}

#[tokio::test]
async fn a_stale_version_gets_every_option_including_empty_ones_and_the_current_none() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.device(ID).await;
        c.global(&[
            ("enable-file-transfer", "N"),
            ("allow-remote-config-modification", ""),
        ])
        .await;
        let current = c.strategies.cache().version();

        // Act
        let stale = send(&c, &[], 0).await;
        let up_to_date = send(&c, &[], current).await;
        let newer = send(&c, &[], current + 1).await;

        // Assert
        let b = db.backend;
        let push = stale.strategy.unwrap();
        assert_eq!(push.modified_at, current, "{b}");
        let expected = opts(&[
            ("enable-file-transfer", "N"),
            ("allow-remote-config-modification", ""),
        ]);
        assert_eq!(push.config_options, expected, "{b}: empty values stay");
        assert_eq!(up_to_date.strategy, None, "{b}");
        assert!(
            newer.strategy.is_some(),
            "{b}: != not <, a higher value is refetched too"
        );
    })
    .await;
}

#[tokio::test]
async fn a_removed_key_is_sent_empty_until_confirmed_and_then_never_again() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.device(ID).await;
        let strategy = c.global(&[("a", "1"), ("b", "2")]).await;
        let mut client = Client { modified_at: 0 };
        client.beat(&c).await;
        client.beat(&c).await;
        c.set_options(strategy, &[("a", "1")]).await;

        // Act
        let lost = send(&c, &[], client.modified_at).await;
        let removal = client.beat(&c).await;
        let confirmed = client.beat(&c).await;
        c.set_options(strategy, &[("a", "3")]).await;
        let next_change = client.beat(&c).await;

        // Assert
        let b = db.backend;
        assert_eq!(options(&lost), opts(&[("a", "1"), ("b", "")]), "{b}");
        assert_eq!(
            options(&removal),
            opts(&[("a", "1"), ("b", "")]),
            "{b}: resent, the first reply was lost"
        );
        assert_eq!(confirmed.strategy, None, "{b}");
        assert_eq!(
            options(&next_change),
            opts(&[("a", "3")]),
            "{b}: b is gone for good"
        );
    })
    .await;
}

#[tokio::test]
async fn keys_of_a_device_group_it_left_are_deleted() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.device(ID).await;
        let alice = c.user("alice").await.id;
        let group = c.devices.create_group("Office", "").await.unwrap();
        c.preassign(alice, group).await;
        let created = c
            .strategies
            .create(NewStrategy::new("office", opts(&[("k", "v")])));
        let office = created.await.unwrap();
        c.strategies
            .assign(Target::DeviceGroup(group), office.id)
            .await
            .unwrap();
        let mut client = Client { modified_at: 0 };
        let joined = client.beat(&c).await;
        client.beat(&c).await;

        // Act
        c.devices.delete_group(group).await.unwrap();
        let left = client.beat(&c).await;

        // Assert
        let b = db.backend;
        assert_eq!(options(&joined), opts(&[("k", "v")]), "{b}");
        assert_eq!(options(&left), opts(&[("k", "")]), "{b}");
    })
    .await;
}

/// Deleting a device drops its row and its direct assignment; the running client keeps the
/// options it was sent and registers again as a new row. The keys sent to it are kept by
/// RustDesk id, so the new row's first strategy still deletes them.
#[tokio::test]
async fn a_deleted_and_re_registered_device_still_gets_its_old_keys_deleted() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let old_row = c.device(ID).await;
        let created = c
            .strategies
            .create(NewStrategy::new("own", opts(&[("k", "v")])));
        let own = created.await.unwrap();
        c.strategies
            .assign(Target::Device(old_row), own.id)
            .await
            .unwrap();
        let mut client = Client { modified_at: 0 };
        let applied = client.beat(&c).await;
        client.beat(&c).await;

        // Act
        assert!(c.devices.delete(old_row).await.unwrap());
        let new_row = c.device(ID).await;
        let after = client.beat(&c).await;

        // Assert
        let b = db.backend;
        assert_ne!(new_row, old_row, "{b}");
        assert_eq!(options(&applied), opts(&[("k", "v")]), "{b}");
        assert_eq!(options(&after), opts(&[("k", "")]), "{b}");
    })
    .await;
}

/// The key set survives a restart: a key removed while the device was offline, with a
/// restart in between, is still deleted. A restart moves the version, so the device refetches.
#[tokio::test]
async fn a_restart_keeps_the_keys_to_delete() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.device(ID).await;
        let strategy = c.global(&[("a", "1"), ("b", "2")]).await;
        let mut client = Client { modified_at: 0 };
        client.beat(&c).await;
        client.beat(&c).await;
        c.set_options(strategy, &[("a", "1")]).await;
        let clock = Arc::new(ManualClock::at(T0_MS + 3_600_000));

        // Act
        let restarted = ctx_on(db.conn.clone(), clock).await;
        let after_restart = client.beat(&restarted).await;
        let confirmed = client.beat(&restarted).await;

        // Assert
        let b = db.backend;
        assert_eq!(
            options(&after_restart),
            opts(&[("a", "1"), ("b", "")]),
            "{b}"
        );
        assert_eq!(confirmed.strategy, None, "{b}");
    })
    .await;
}

/// A restart before the device confirmed a removal sends the `""` again (the stored set only
/// shrinks on confirmation).
#[tokio::test]
async fn a_restart_before_the_confirmation_sends_the_delete_again() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.device(ID).await;
        let strategy = c.global(&[("a", "1"), ("b", "2")]).await;
        let mut client = Client { modified_at: 0 };
        client.beat(&c).await;
        client.beat(&c).await;
        c.set_options(strategy, &[]).await;
        let unconfirmed = client.beat(&c).await;
        let clock = Arc::new(ManualClock::at(T0_MS + 3_600_000));

        // Act
        let restarted = ctx_on(db.conn.clone(), clock).await;
        let again = client.beat(&restarted).await;
        client.beat(&restarted).await;
        restarted.set_options(strategy, &[("c", "1")]).await;
        let later = client.beat(&restarted).await;

        // Assert
        let b = db.backend;
        assert_eq!(options(&unconfirmed), opts(&[("a", ""), ("b", "")]), "{b}");
        assert_eq!(options(&again), opts(&[("a", ""), ("b", "")]), "{b}");
        assert_eq!(options(&later), opts(&[("c", "1")]), "{b}");
    })
    .await;
}

/// 02 §4.1: once the identity is cached and the client holds the current version, a
/// heartbeat runs no statement at all (no write, no read). The one write after a strategy
/// change is the confirmation shrinking the stored keys.
#[tokio::test]
async fn the_steady_path_runs_no_statement() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let (conn, log) = counted(&db.conn);
        let c = ctx_on(conn, Arc::new(ManualClock::at(T0_MS))).await;
        c.device(ID).await;
        c.report(report(ID, sysinfo("pc"), Presets::default()))
            .await;
        let strategy = c.global(&[("a", "1"), ("b", "2")]).await;
        let mut client = Client { modified_at: 0 };
        client.beat(&c).await;
        client.beat(&c).await;
        c.set_options(strategy, &[("a", "1")]).await;
        client.beat(&c).await;
        take(&log);

        // Act
        let confirming = client.beat(&c).await;
        let confirmation = take(&log);
        let mut steady = Vec::new();
        for _ in 0..5 {
            steady.push(client.beat(&c).await);
        }
        let steady_statements = take(&log);

        // Assert
        let b = db.backend;
        assert_eq!(confirming, HeartbeatReply::default(), "{b}");
        assert_eq!(confirmation.len(), 1, "{b}: {confirmation:?}");
        assert!(
            confirmation[0].starts_with("UPDATE"),
            "{b}: {confirmation:?}"
        );
        assert!(
            steady.iter().all(|r| *r == HeartbeatReply::default()),
            "{b}"
        );
        assert_eq!(steady_statements, Vec::<String>::new(), "{b}");
        let seen = c.devices.presence().get(ID).unwrap().last_seen_ms;
        assert_eq!(seen, Some(c.clock.now_ms()), "{b}: presence still updated");
    })
    .await;
}
