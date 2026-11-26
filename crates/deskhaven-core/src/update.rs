//! On demand only: an instance nobody administers never connects out. Only the response's
//! `tag_name` is trusted, and only as `v` + a plain semver version; the release link is built here,
//! never taken from the response.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use deskhaven_config::settings::UPDATES_CHECK;
use openidconnect::HttpRequest;
use openidconnect::http::{Method, StatusCode, header};
use semver::Version;
use serde::Deserialize;

use crate::build::{SOURCE_URL, VERSION};
use crate::clock::SharedClock;
use crate::oidc::{FetchError, HttpFetch};
use crate::settings::SettingsService;

const MAX_TAG_LEN: usize = 32;
const RECHECK_MS: i64 = 24 * 60 * 60 * 1000;
/// GitHub refuses requests without one; nothing about the instance goes in it.
const USER_AGENT: &str = "deskhaven";
const ACCEPT: &str = "application/vnd.github+json";
const GITHUB: &str = "https://github.com/";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableUpdate {
    pub version: Version,
    pub release_url: String,
}

pub fn parse_tag(tag: &str) -> Option<Version> {
    if tag.len() > MAX_TAG_LEN {
        return None;
    }
    let version = Version::parse(tag.strip_prefix('v')?).ok()?;
    (version.pre.is_empty() && version.build.is_empty()).then_some(version)
}

pub fn latest_release_url(source_url: &str) -> Option<String> {
    let (owner, repo) = source_url.strip_prefix(GITHUB)?.split_once('/')?;
    let plain = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (plain(owner) && plain(repo))
        .then(|| format!("https://api.github.com/repos/{owner}/{repo}/releases/latest"))
}

/// Never carries anything from the response but its status.
#[derive(Debug, thiserror::Error)]
enum CheckError {
    #[error("cannot build the request")]
    Request,
    #[error(transparent)]
    Fetch(#[from] FetchError),
    #[error("unexpected status {0}")]
    Status(StatusCode),
    #[error("the response is not a release")]
    NotARelease,
    #[error("the release tag is not a plain version")]
    BadTag,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
}

pub struct UpdateService {
    settings: SettingsService,
    clock: SharedClock,
    fetch: Arc<dyn HttpFetch>,
    /// `None` when [`SOURCE_URL`] is not a GitHub repository: the check never runs.
    target: Option<Target>,
    cache: Mutex<Cache>,
}

struct Target {
    api_url: String,
    current: Version,
}

#[derive(Default)]
struct Cache {
    latest: Option<Version>,
    checked_at: Option<i64>,
    in_flight: bool,
}

impl Cache {
    fn due(&self, now: i64) -> bool {
        !self.in_flight && self.checked_at.is_none_or(|at| now - at >= RECHECK_MS)
    }
}

impl UpdateService {
    pub fn new(settings: SettingsService, clock: SharedClock, fetch: Arc<dyn HttpFetch>) -> Self {
        let target = latest_release_url(SOURCE_URL)
            .zip(Version::parse(VERSION).ok())
            .map(|(api_url, current)| Target { api_url, current });
        Self::with_target(settings, clock, fetch, target)
    }

    fn with_target(
        settings: SettingsService,
        clock: SharedClock,
        fetch: Arc<dyn HttpFetch>,
        target: Option<Target>,
    ) -> Self {
        Self {
            settings,
            clock,
            fetch,
            target,
            cache: Mutex::new(Cache::default()),
        }
    }

    /// Never waits for the check. Must be called inside a Tokio runtime.
    pub fn available(self: &Arc<Self>) -> Option<AvailableUpdate> {
        if !self.settings.snapshot().get(&UPDATES_CHECK) {
            return None;
        }
        let target = self.target.as_ref()?;
        let latest = {
            let mut cache = self.cache();
            if cache.due(self.clock.elapsed_ms()) {
                cache.in_flight = true;
                tokio::spawn(Arc::clone(self).refresh(target.api_url.clone()));
            }
            cache.latest.clone()
        };
        latest
            .filter(|latest| *latest > target.current)
            .map(|version| AvailableUpdate {
                release_url: format!("{SOURCE_URL}/releases/tag/v{version}"),
                version,
            })
    }

    async fn refresh(self: Arc<Self>, api_url: String) {
        let _finish = Finish(&self);
        match self.check(&api_url).await {
            Ok(version) => {
                tracing::debug!(%version, "update check: latest release");
                self.cache().latest = Some(version);
            }
            Err(error) => {
                tracing::info!(%error, setting = UPDATES_CHECK.key(), "update check failed");
            }
        }
    }

    async fn check(&self, api_url: &str) -> Result<Version, CheckError> {
        let request: HttpRequest = openidconnect::http::Request::builder()
            .method(Method::GET)
            .uri(api_url)
            .header(header::USER_AGENT, USER_AGENT)
            .header(header::ACCEPT, ACCEPT)
            .body(Vec::new())
            .map_err(|_| CheckError::Request)?;
        let response = self.fetch.fetch(request).await?;
        if response.status() != StatusCode::OK {
            return Err(CheckError::Status(response.status()));
        }
        let release: Release =
            serde_json::from_slice(response.body()).map_err(|_| CheckError::NotARelease)?;
        parse_tag(&release.tag_name).ok_or(CheckError::BadTag)
    }

    fn cache(&self) -> MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Without it a dead refresh would leave `in_flight` set and no check would ever run again.
struct Finish<'a>(&'a UpdateService);

impl Drop for Finish<'_> {
    fn drop(&mut self) {
        let mut cache = self.0.cache();
        cache.checked_at = Some(self.0.clock.elapsed_ms());
        cache.in_flight = false;
    }
}

#[cfg(test)]
mod tests;
