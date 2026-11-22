//! The update check: whether a newer deskhaven release than this build is out.
//!
//! On demand only: [`UpdateService::available`] (an admin's console asking) answers from the
//! cache and, when the cache is empty or a day old and `updates.check` is on, starts one
//! background request to the GitHub releases API of [`SOURCE_URL`]. An instance nobody
//! administers never connects out. Only the response's `tag_name` is trusted, and only when
//! it is `v` + a plain semver version; the release page link is built here from that
//! version, never taken from the response.

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

/// Longest accepted `tag_name`.
const MAX_TAG_LEN: usize = 32;
/// How long a check, successful or not, is good for.
const RECHECK_MS: i64 = 24 * 60 * 60 * 1000;
/// GitHub refuses requests without one; nothing about the instance goes in it.
const USER_AGENT: &str = "deskhaven";
const ACCEPT: &str = "application/vnd.github+json";
const GITHUB: &str = "https://github.com/";

/// A release newer than the running build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableUpdate {
    pub version: Version,
    /// Its release page under [`SOURCE_URL`].
    pub release_url: String,
}

/// `"v0.1.3"` -> 0.1.3. Rejects prereleases, build metadata, over-long tags and anything
/// else.
pub fn parse_tag(tag: &str) -> Option<Version> {
    if tag.len() > MAX_TAG_LEN {
        return None;
    }
    let version = Version::parse(tag.strip_prefix('v')?).ok()?;
    (version.pre.is_empty() && version.build.is_empty()).then_some(version)
}

/// The GitHub API URL of the latest release of the repository at `source_url`
/// (`https://github.com/<owner>/<repo>`); `None` for anything else.
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

/// Why one check failed. Never carries anything from the response but its status.
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

/// The update check (see the module docs). Shared as `Arc<UpdateService>`.
pub struct UpdateService {
    settings: SettingsService,
    clock: SharedClock,
    fetch: Arc<dyn HttpFetch>,
    /// `None` when [`SOURCE_URL`] is not a GitHub repository (a fork hosted elsewhere): the
    /// check never runs.
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
    /// Monotonic ms of the last finished check.
    checked_at: Option<i64>,
    in_flight: bool,
}

impl Cache {
    fn due(&self, now: i64) -> bool {
        !self.in_flight && self.checked_at.is_none_or(|at| now - at >= RECHECK_MS)
    }
}

impl UpdateService {
    /// Checks this build ([`VERSION`]) against the releases of [`SOURCE_URL`].
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

    /// The newer release the cache knows of; `None` while `updates.check` is off (nothing is
    /// sent then). Starts one background check when the cache is empty or a day old, but
    /// never waits for it. Must be called inside a Tokio runtime.
    pub fn available(self: &Arc<Self>) -> Option<AvailableUpdate> {
        if !self.settings.snapshot().get(&UPDATES_CHECK) {
            return None;
        }
        let target = self.target.as_ref()?;
        let latest = {
            let mut cache = self.cache();
            if cache.due(self.clock.elapsed_ms()) {
                cache.in_flight = true;
                tokio::spawn(Arc::clone(self).refresh());
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

    async fn refresh(self: Arc<Self>) {
        let result = match &self.target {
            Some(target) => self.check(&target.api_url).await,
            None => Err(CheckError::Request),
        };
        let mut cache = self.cache();
        match result {
            Ok(version) => {
                tracing::debug!(%version, "update check: latest release");
                cache.latest = Some(version);
            }
            Err(error) => {
                tracing::info!(%error, setting = UPDATES_CHECK.key(), "update check failed");
            }
        }
        cache.checked_at = Some(self.clock.elapsed_ms());
        cache.in_flight = false;
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

#[cfg(test)]
mod tests;
