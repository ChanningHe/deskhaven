use std::sync::{Mutex as StdMutex, PoisonError};
use std::time::Duration;

use deskhaven_config::settings::{Change, PinSet};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use openidconnect::http::{Method, Response, StatusCode, header};
use openidconnect::{HttpRequest, HttpResponse};
use serde_json::json;
use tokio::sync::Semaphore;

use super::*;
use crate::build::SOURCE_URL;
use crate::clock::ManualClock;
use crate::crypto::{MasterKey, SecretBox};
use crate::oidc::{FetchError, FetchFuture};
use crate::settings::UpdateRequest;

const T0_MS: i64 = 1_700_000_000_000;
const HOUR: Duration = Duration::from_secs(60 * 60);
const API: &str = "https://api.github.test/repos/o/r/releases/latest";

type Respond = Box<dyn Fn() -> Result<HttpResponse, FetchError> + Send + Sync>;

/// Answers with `respond` once its gate lets a request through; records every request.
struct FakeFetch {
    respond: StdMutex<Respond>,
    requests: StdMutex<Vec<HttpRequest>>,
    gate: Semaphore,
}

impl FakeFetch {
    fn open(
        respond: impl Fn() -> Result<HttpResponse, FetchError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            respond: StdMutex::new(Box::new(respond)),
            requests: StdMutex::new(Vec::new()),
            gate: Semaphore::new(Semaphore::MAX_PERMITS),
        }
    }

    fn gated(
        respond: impl Fn() -> Result<HttpResponse, FetchError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            gate: Semaphore::new(0),
            ..Self::open(respond)
        }
    }

    fn calls(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    fn reply_with(
        &self,
        respond: impl Fn() -> Result<HttpResponse, FetchError> + Send + Sync + 'static,
    ) {
        *self.respond.lock().unwrap_or_else(PoisonError::into_inner) = Box::new(respond);
    }
}

impl HttpFetch for FakeFetch {
    fn fetch(&self, request: HttpRequest) -> FetchFuture<'_> {
        self.requests.lock().unwrap().push(request);
        Box::pin(async move {
            let _permit = self.gate.acquire().await;
            // A panicking `respond` poisons the lock; later replies still go through.
            (self.respond.lock().unwrap_or_else(PoisonError::into_inner))()
        })
    }
}

fn reply(status: StatusCode, body: &str) -> Result<HttpResponse, FetchError> {
    let mut response = Response::new(body.as_bytes().to_vec());
    *response.status_mut() = status;
    Ok(response)
}

/// A release whose other fields try to steer the console elsewhere.
fn release(tag: &str) -> Result<HttpResponse, FetchError> {
    let body = json!({ "tag_name": tag, "html_url": "https://evil.example/", "name": "v99.0.0" });
    reply(StatusCode::OK, &body.to_string())
}

struct Fixture {
    service: Arc<UpdateService>,
    fetch: Arc<FakeFetch>,
    clock: Arc<ManualClock>,
    settings: SettingsService,
}

async fn fixture(db: &TestDb, current: &str, fetch: FakeFetch) -> Fixture {
    let target = Target {
        api_url: API.to_owned(),
        current: Version::parse(current).unwrap(),
    };
    fixture_for(db, Some(target), fetch).await
}

async fn fixture_for(db: &TestDb, target: Option<Target>, fetch: FakeFetch) -> Fixture {
    let crypto = Arc::new(SecretBox::new(&MasterKey::from_bytes(&[7; 32])));
    let (settings, _) = SettingsService::load(db.conn.clone(), crypto, PinSet::empty())
        .await
        .unwrap();
    let clock = Arc::new(ManualClock::at(T0_MS));
    let fetch = Arc::new(fetch);
    let service = Arc::new(UpdateService::with_target(
        settings.clone(),
        Arc::clone(&clock) as SharedClock,
        Arc::clone(&fetch) as Arc<dyn HttpFetch>,
        target,
    ));
    Fixture {
        service,
        fetch,
        clock,
        settings,
    }
}

async fn until(what: &str, done: impl Fn() -> bool) {
    for _ in 0..1000 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("never saw: {what}");
}

/// Waits until no refresh is running.
async fn settle(service: &UpdateService) {
    until("the refresh finish", || {
        !service.cache.lock().unwrap().in_flight
    })
    .await;
}

/// Asks, lets the refresh it starts finish, and asks again.
async fn checked(f: &Fixture) -> Option<AvailableUpdate> {
    f.service.available();
    settle(&f.service).await;
    f.service.available()
}

async fn set_check(settings: &SettingsService, on: bool) {
    let version = settings.snapshot().view("updates.check").unwrap().version;
    let change = Change {
        key: "updates.check".to_owned(),
        value: json!(on),
        version,
    };
    let request = UpdateRequest {
        changes: vec![change],
        actor: None,
        reason: None,
    };
    settings.update(request).await.unwrap();
}

fn v(text: &str) -> Version {
    Version::parse(text).unwrap()
}

#[test]
fn only_a_v_prefixed_plain_version_is_a_release_tag() {
    let accepted = [
        ("v0.1.2", "0.1.2"),
        ("v10.20.30", "10.20.30"),
        ("v0.0.0", "0.0.0"),
        (
            "v123456789.1234567890.1234567890",
            "123456789.1234567890.1234567890",
        ),
    ];
    for (tag, version) in accepted {
        assert_eq!(parse_tag(tag), Some(v(version)), "{tag:?}");
    }
    let rejected = [
        "",
        "v",
        "0.1.2",
        "V0.1.2",
        "vv0.1.2",
        "v0.1",
        "v0.1.2.3",
        "v01.2.3",
        "v-1.2.3",
        "v0.1.2-rc.1",
        "v0.1.2+build.5",
        "v0.1.2-rc.1+build.5",
        " v0.1.2",
        "v0.1.2 ",
        "v0.1.2\n",
        "v1234567890.1234567890.1234567890",
        "release-0.1.2",
        "v0.1.2/../evil",
    ];
    for tag in rejected {
        assert_eq!(parse_tag(tag), None, "{tag:?}");
    }
}

#[test]
fn the_api_url_derives_from_a_github_repository_url_only() {
    assert_eq!(
        latest_release_url("https://github.com/ChanningHe/deskhaven").as_deref(),
        Some("https://api.github.com/repos/ChanningHe/deskhaven/releases/latest")
    );
    assert!(latest_release_url(SOURCE_URL).is_some());
    for url in [
        "https://gitlab.com/o/r",
        "http://github.com/o/r",
        "https://github.com/o",
        "https://github.com/o/r/",
        "https://github.com/o/r/x",
        "https://github.com//r",
        "https://github.com/o/r?x=1",
        "https://github.com/o/r#x",
        "https://github.com.evil.example/o/r",
    ] {
        assert_eq!(latest_release_url(url), None, "{url}");
    }
}

#[tokio::test]
async fn a_newer_release_is_offered_with_a_link_built_from_its_version() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, "0.1.1", FakeFetch::open(|| release("v0.2.0"))).await;

        assert_eq!(f.service.available(), None, "answers from the empty cache");
        settle(&f.service).await;

        let expected = AvailableUpdate {
            version: v("0.2.0"),
            release_url: format!("{SOURCE_URL}/releases/tag/v0.2.0"),
        };
        assert_eq!(f.service.available(), Some(expected));
    })
    .await;
}

#[tokio::test]
async fn an_equal_or_older_release_is_not_offered() {
    each_migrated_backend(async |db: &TestDb| {
        for tag in ["v0.1.1", "v0.1.0", "v0.0.9"] {
            let f = fixture(db, "0.1.1", FakeFetch::open(move || release(tag))).await;
            assert_eq!(checked(&f).await, None, "{tag}");
            assert_eq!(f.fetch.calls(), 1);
        }
    })
    .await;
}

#[tokio::test]
async fn a_prerelease_build_is_behind_its_release() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, "0.1.2-rc.1", FakeFetch::open(|| release("v0.1.2"))).await;
        let offered = checked(&f).await.unwrap();
        assert_eq!(offered.version, v("0.1.2"));
    })
    .await;
}

#[tokio::test]
async fn the_request_is_a_bare_get_with_a_fixed_user_agent() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, "0.1.1", FakeFetch::open(|| release("v0.1.1"))).await;
        checked(&f).await;

        let requests = f.fetch.requests.lock().unwrap();
        let [request] = requests.as_slice() else {
            panic!("{} requests", requests.len());
        };
        assert_eq!(request.method(), Method::GET);
        assert_eq!(request.uri().to_string(), API);
        assert_eq!(request.headers()[header::USER_AGENT], "deskhaven");
        assert_eq!(
            request.headers()[header::ACCEPT],
            "application/vnd.github+json"
        );
        assert_eq!(request.headers().len(), 2);
        assert!(request.body().is_empty());
    })
    .await;
}

fn failures() -> Vec<(&'static str, Respond)> {
    vec![
        (
            "server error",
            Box::new(|| reply(StatusCode::INTERNAL_SERVER_ERROR, "{}")),
        ),
        (
            "rate limited",
            Box::new(|| reply(StatusCode::FORBIDDEN, r#"{"tag_name":"v9.0.0"}"#)),
        ),
        (
            "redirect",
            Box::new(|| reply(StatusCode::MOVED_PERMANENTLY, "")),
        ),
        ("too large", Box::new(|| Err(FetchError::TooLarge))),
        ("not JSON", Box::new(|| reply(StatusCode::OK, "<html>"))),
        (
            "no tag",
            Box::new(|| reply(StatusCode::OK, r#"{"name":"v9.0.0"}"#)),
        ),
        (
            "tag not a string",
            Box::new(|| reply(StatusCode::OK, r#"{"tag_name":9}"#)),
        ),
        ("prerelease tag", Box::new(|| release("v9.0.0-rc.1"))),
        ("tag without v", Box::new(|| release("9.0.0"))),
    ]
}

#[tokio::test]
async fn a_failed_check_keeps_the_last_known_release() {
    each_migrated_backend(async |db: &TestDb| {
        for (name, failure) in failures() {
            let f = fixture(db, "0.1.1", FakeFetch::open(|| release("v0.2.0"))).await;
            assert!(checked(&f).await.is_some(), "{name}");

            f.fetch.reply_with(failure);
            f.clock.advance(25 * HOUR);
            let offered = checked(&f).await;

            assert_eq!(f.fetch.calls(), 2, "{name}");
            assert_eq!(offered.map(|u| u.version), Some(v("0.2.0")), "{name}");
        }
    })
    .await;
}

#[tokio::test]
async fn a_failed_first_check_offers_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        for (name, failure) in failures() {
            let f = fixture(db, "0.1.1", FakeFetch::open(failure)).await;
            assert_eq!(checked(&f).await, None, "{name}");
        }
    })
    .await;
}

/// What a failure logs is its error: nothing of the response but the status.
#[tokio::test]
async fn a_failure_reports_nothing_from_the_response() {
    each_migrated_backend(async |db: &TestDb| {
        let leaks: Vec<(Respond, &str)> = vec![
            (Box::new(|| reply(StatusCode::BAD_GATEWAY, "SECRET")), "502"),
            (
                Box::new(|| reply(StatusCode::OK, r#"{"tag_name":7,"x":"SECRET"}"#)),
                "not a release",
            ),
            (Box::new(|| release("SECRET")), "not a plain version"),
            (Box::new(|| release("v9.9.9-SECRET")), "not a plain version"),
        ];
        let f = fixture(db, "0.1.1", FakeFetch::open(|| release("v0.1.1"))).await;
        for (respond, expected) in leaks {
            f.fetch.reply_with(respond);
            let error = f.service.check(API).await.unwrap_err().to_string();
            assert!(error.contains(expected), "{error}");
            assert!(!error.contains("SECRET"), "{error}");
        }
    })
    .await;
}

#[tokio::test]
async fn nothing_is_requested_while_the_check_is_off() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, "0.1.1", FakeFetch::open(|| release("v0.2.0"))).await;
        set_check(&f.settings, false).await;

        for _ in 0..3 {
            assert_eq!(f.service.available(), None);
        }
        // Lets a refresh, had one been spawned, send its request.
        tokio::task::yield_now().await;

        assert_eq!(f.fetch.calls(), 0);
        assert!(!f.service.cache.lock().unwrap().in_flight);
    })
    .await;
}

#[tokio::test]
async fn turning_the_check_off_hides_a_known_release_at_once() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, "0.1.1", FakeFetch::open(|| release("v0.2.0"))).await;
        assert!(checked(&f).await.is_some());

        set_check(&f.settings, false).await;

        assert_eq!(f.service.available(), None);
        assert_eq!(f.fetch.calls(), 1);
    })
    .await;
}

#[tokio::test]
async fn the_release_is_asked_for_at_most_once_a_day() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, "0.1.1", FakeFetch::open(|| release("v0.2.0"))).await;
        checked(&f).await;
        for _ in 0..3 {
            assert!(f.service.available().is_some());
        }
        f.clock.advance(23 * HOUR);
        checked(&f).await;
        assert_eq!(f.fetch.calls(), 1);

        f.clock.advance(2 * HOUR);
        checked(&f).await;
        assert_eq!(f.fetch.calls(), 2);
    })
    .await;
}

#[tokio::test]
async fn a_failed_check_is_retried_only_after_a_day() {
    each_migrated_backend(async |db: &TestDb| {
        let fetch = FakeFetch::open(|| reply(StatusCode::SERVICE_UNAVAILABLE, ""));
        let f = fixture(db, "0.1.1", fetch).await;
        checked(&f).await;
        f.clock.advance(23 * HOUR);
        checked(&f).await;
        assert_eq!(f.fetch.calls(), 1);

        f.clock.advance(2 * HOUR);
        checked(&f).await;
        assert_eq!(f.fetch.calls(), 2);
    })
    .await;
}

#[tokio::test]
async fn concurrent_askers_share_one_request() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, "0.1.1", FakeFetch::gated(|| release("v0.2.0"))).await;

        for _ in 0..5 {
            assert_eq!(f.service.available(), None);
        }
        until("the request", || f.fetch.calls() > 0).await;
        assert_eq!(f.fetch.calls(), 1);
        assert!(f.service.cache.lock().unwrap().in_flight);

        f.fetch.gate.add_permits(1);
        settle(&f.service).await;

        assert_eq!(f.fetch.calls(), 1);
        assert!(f.service.available().is_some());
    })
    .await;
}

#[tokio::test]
async fn without_a_target_nothing_is_requested() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture_for(db, None, FakeFetch::open(|| release("v0.2.0"))).await;

        for _ in 0..3 {
            assert_eq!(f.service.available(), None);
        }
        // Lets a refresh, had one been spawned, send its request.
        tokio::task::yield_now().await;

        assert_eq!(f.fetch.calls(), 0);
        assert!(!f.service.cache.lock().unwrap().in_flight);
    })
    .await;
}

/// A refresh that dies inside the request still ends: the next one comes a day later.
#[tokio::test]
async fn a_refresh_that_panics_is_retried_after_a_day() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, "0.1.1", FakeFetch::open(|| panic!("fetch exploded"))).await;
        assert_eq!(checked(&f).await, None);

        f.fetch.reply_with(|| release("v0.2.0"));
        f.clock.advance(23 * HOUR);
        checked(&f).await;
        assert_eq!(f.fetch.calls(), 1);

        f.clock.advance(2 * HOUR);
        let offered = checked(&f).await;
        assert_eq!(f.fetch.calls(), 2);
        assert_eq!(offered.map(|u| u.version), Some(v("0.2.0")));
    })
    .await;
}
