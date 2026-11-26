#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::Method;
use common::web_client::{
    ALICE_DEVICE, OWNER_DEVICE, SESSIONS, candidates_path, enabled_harness,
    enabled_harness_on_clock, open, opened, setup,
};
use deskhaven_core::clock::{Clock, ManualClock, SystemClock};
use deskhaven_core::users::Role;
use deskhaven_core::web_client::{
    CandidateRejected, MAX_SESSIONS_PER_USER, SESSION_TTL, WEB_SESSIONS_PER_DEVICE_PER_MINUTE,
    WEB_SESSIONS_PER_USER_PER_MINUTE,
};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::json;

#[tokio::test]
async fn candidates_travel_both_ways_and_an_idle_poll_ends_empty() {
    each_migrated_backend(async |db: &TestDb| {
        let h = enabled_harness(db).await;
        let s = setup(&h).await;
        let id = opened(&h, &s.alice, ALICE_DEVICE).await;
        let route = s.fake.take().pop().unwrap();
        let path = candidates_path(&id);
        route
            .from_device
            .send("candidate:dev-0".to_owned())
            .unwrap();

        let sent = h
            .write(
                Method::POST,
                &path,
                &s.alice,
                json!({ "candidate": "candidate:web-0" }),
            )
            .await;
        let first = h.get(&format!("{path}?after=0"), &s.alice).await;
        let started = Instant::now();
        let idle = h.get(&format!("{path}?after=1&wait=1"), &s.alice).await;
        let waited = started.elapsed();
        let poll = {
            let (h, alice) = (&h, &s.alice);
            let path = format!("{path}?after=1");
            async move { h.get(&path, alice).await }
        };
        let push = async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            route
                .from_device
                .send("candidate:dev-1".to_owned())
                .unwrap();
        };
        let (second, ()) = tokio::join!(poll, push);

        let b = db.backend;
        assert_eq!(sent.status, 204, "{b}: {:?}", sent.body);
        assert_eq!(*route.to_device.lock().unwrap(), ["candidate:web-0"]);
        assert_eq!(
            first.body,
            json!({ "candidates": ["candidate:dev-0"], "next": 1 }),
            "{b}"
        );
        assert_eq!(idle.body, json!({ "candidates": [], "next": 1 }), "{b}");
        assert!(waited >= Duration::from_secs(1), "{b}: waited {waited:?}");
        assert_eq!(
            second.body,
            json!({ "candidates": ["candidate:dev-1"], "next": 2 }),
            "{b}"
        );
    })
    .await;
}

#[tokio::test]
async fn a_closed_route_and_refused_candidates_answer_their_codes() {
    each_migrated_backend(async |db: &TestDb| {
        let h = enabled_harness(db).await;
        let s = setup(&h).await;
        let id = opened(&h, &s.owner, OWNER_DEVICE).await;
        drop(s.fake.take());
        let path = candidates_path(&id);
        let send = async |candidate: &str| {
            let body = json!({ "candidate": candidate });
            h.write(Method::POST, &path, &s.owner, body).await
        };

        let gone = h.get(&path, &s.owner).await;
        s.fake.reject_candidates(CandidateRejected::Invalid);
        let rejected_id = opened(&h, &s.owner, OWNER_DEVICE).await;
        let invalid = h
            .write(
                Method::POST,
                &candidates_path(&rejected_id),
                &s.owner,
                json!({ "candidate": "x" }),
            )
            .await;
        s.fake.reject_candidates(CandidateRejected::OverBudget);
        let budget_id = opened(&h, &s.owner, OWNER_DEVICE).await;
        let over = h
            .write(
                Method::POST,
                &candidates_path(&budget_id),
                &s.owner,
                json!({ "candidate": "x" }),
            )
            .await;
        let too_long = send(&"x".repeat(2049)).await;

        let b = db.backend;
        assert_eq!(
            (gone.status.as_u16(), gone.code()),
            (410, "session_closed"),
            "{b}"
        );
        assert_eq!(
            (invalid.status.as_u16(), invalid.code()),
            (400, "invalid_candidate"),
            "{b}"
        );
        assert_eq!(
            (over.status.as_u16(), over.code()),
            (429, "too_many_candidates"),
            "{b}"
        );
        assert_eq!(too_long.status, 422, "{b}");
    })
    .await;
}

#[tokio::test]
async fn a_session_answers_only_the_user_who_opened_it() {
    each_migrated_backend(async |db: &TestDb| {
        let h = enabled_harness(db).await;
        let s = setup(&h).await;
        let id = opened(&h, &s.alice, ALICE_DEVICE).await;
        let path = candidates_path(&id);
        let session = format!("{SESSIONS}/{id}");

        let poll = h.get(&format!("{path}?wait=0"), &s.owner).await;
        let send = h
            .write(Method::POST, &path, &s.owner, json!({ "candidate": "c" }))
            .await;
        let close = h.act(Method::DELETE, &session, &s.owner).await;
        let own_close = h.act(Method::DELETE, &session, &s.alice).await;
        let after_close = h.get(&format!("{path}?wait=0"), &s.alice).await;
        let unknown = h
            .act(Method::DELETE, &format!("{SESSIONS}/nope"), &s.alice)
            .await;

        let b = db.backend;
        for reply in [&poll, &send, &close, &after_close, &unknown] {
            assert_eq!(reply.status, 404, "{b}: {:?}", reply.body);
        }
        assert_eq!(own_close.status, 204, "{b}");
    })
    .await;
}

#[tokio::test]
async fn sessions_are_capped_per_user_and_expire() {
    each_migrated_backend(async |db: &TestDb| {
        let clock = Arc::new(ManualClock::at(SystemClock.now_ms()));
        let h = enabled_harness_on_clock(db, clock.clone()).await;
        let s = setup(&h).await;
        let mut ids = Vec::new();
        for _ in 0..MAX_SESSIONS_PER_USER {
            ids.push(opened(&h, &s.alice, ALICE_DEVICE).await);
        }

        let over = open(&h, &s.alice, ALICE_DEVICE, false).await;
        let other_user = open(&h, &s.owner, OWNER_DEVICE, false).await;
        let closed = h
            .act(Method::DELETE, &format!("{SESSIONS}/{}", ids[0]), &s.alice)
            .await;
        let after_close = open(&h, &s.alice, ALICE_DEVICE, false).await;
        clock.advance(SESSION_TTL);
        let expired = h
            .get(&format!("{}?wait=0", candidates_path(&ids[1])), &s.alice)
            .await;
        let after_expiry = open(&h, &s.alice, ALICE_DEVICE, false).await;

        let b = db.backend;
        assert_eq!(
            (over.status.as_u16(), over.code()),
            (429, "too_many_sessions"),
            "{b}"
        );
        assert_eq!(other_user.status, 201, "{b}");
        assert_eq!(closed.status, 204, "{b}");
        assert_eq!(after_close.status, 201, "{b}");
        assert_eq!(expired.status, 404, "{b}");
        assert_eq!(after_expiry.status, 201, "{b}");
    })
    .await;
}

#[tokio::test]
async fn sessions_to_one_device_are_rate_limited() {
    each_migrated_backend(async |db: &TestDb| {
        let h = enabled_harness(db).await;
        let s = setup(&h).await;
        let admin = h.console("admin", Role::Admin).await;
        let mut statuses = Vec::new();

        for i in 0..WEB_SESSIONS_PER_DEVICE_PER_MINUTE {
            let by = if i % 2 == 0 { &s.owner } else { &admin };
            let reply = open(&h, by, ALICE_DEVICE, false).await;
            statuses.push(reply.status.as_u16());
            let id = reply.body["session_id"].as_str().unwrap().to_owned();
            let closed = h.act(Method::DELETE, &format!("{SESSIONS}/{id}"), by).await;
            assert_eq!(closed.status, 204);
        }
        let over = open(&h, &s.alice, ALICE_DEVICE, false).await;

        let b = db.backend;
        assert!(statuses.iter().all(|s| *s == 201), "{b}: {statuses:?}");
        assert_eq!(
            (over.status.as_u16(), over.code()),
            (429, "rate_limited"),
            "{b}"
        );
        assert!(over.headers.contains_key("retry-after"), "{b}");
    })
    .await;
}

#[tokio::test]
async fn sessions_of_one_user_are_rate_limited() {
    each_migrated_backend(async |db: &TestDb| {
        let h = enabled_harness(db).await;
        let s = setup(&h).await;
        let mut statuses = Vec::new();

        for i in 0..WEB_SESSIONS_PER_USER_PER_MINUTE {
            let device = if i % 2 == 0 {
                ALICE_DEVICE
            } else {
                OWNER_DEVICE
            };
            let reply = open(&h, &s.owner, device, false).await;
            statuses.push(reply.status.as_u16());
            let id = reply.body["session_id"].as_str().unwrap().to_owned();
            let closed = h
                .act(Method::DELETE, &format!("{SESSIONS}/{id}"), &s.owner)
                .await;
            assert_eq!(closed.status, 204);
        }
        let over = open(&h, &s.owner, OWNER_DEVICE, false).await;
        let alice = open(&h, &s.alice, ALICE_DEVICE, false).await;

        let b = db.backend;
        assert!(statuses.iter().all(|s| *s == 201), "{b}: {statuses:?}");
        assert_eq!(
            (over.status.as_u16(), over.code()),
            (429, "rate_limited"),
            "{b}"
        );
        assert_eq!(
            alice.status, 201,
            "{b}: another user has a quota of their own"
        );
    })
    .await;
}
