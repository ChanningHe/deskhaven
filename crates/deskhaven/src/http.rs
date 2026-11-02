//! HTTP wiring: listener topology and stack settings from the bootstrap
//! configuration, the routers of each side and the readiness probe.

use std::time::Duration;

use axum::Router;
use axum::http::HeaderValue;
use deskhaven_config::Cidr;
use deskhaven_config::bootstrap::{Bootstrap, Listen};
use deskhaven_http::{
    Apps, ErrorBodies, HttpConfig, ListenConfig, NotReady, ReadinessCheck, TrustedProxies,
    health_routes,
};
use sea_orm::DatabaseConnection;
use tokio_util::sync::CancellationToken;

use crate::log_safe;
use crate::state::AppState;

/// HSTS for https deployments: one year, without `includeSubDomains`
/// (other hosts under the domain are not ours to pin).
pub const HSTS: &str = "max-age=31536000";
/// A readiness probe slower than this counts as not ready.
pub const READY_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const HTTPS_PREFIX: &str = "https://";

/// Stack settings: proxies, CSP of the console, HSTS per listener (the
/// client API, or the single listener, follows `public_url`; a split admin
/// listener follows `admin_public_url`), and the drain share of the shutdown
/// budget.
pub fn http_config(config: &Bootstrap, drain_timeout: Duration) -> HttpConfig {
    HttpConfig {
        trusted_proxies: TrustedProxies::new(config.security.trusted_proxies.iter().map(Cidr::net)),
        drain_timeout,
        content_security_policy: Some(HeaderValue::from_static(deskhaven_web::CSP)),
        strict_transport_security: hsts_for(&config.public_url),
        admin_strict_transport_security: hsts_for(&config.listen.admin_public_url),
        ..HttpConfig::default()
    }
}

fn hsts_for(url: &str) -> Option<HeaderValue> {
    url.get(..HTTPS_PREFIX.len())
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case(HTTPS_PREFIX))
        .then(|| HeaderValue::from_static(HSTS))
}

pub fn listen_config(listen: &Listen) -> ListenConfig {
    ListenConfig {
        client_api: listen.client_api,
        admin: listen.admin_addr(),
        admin_public_url: Some(listen.admin_public_url.clone()).filter(|url| !url.is_empty()),
    }
}

/// Routers of both sides with the API error renderers. Health routes are on
/// both, so a split admin listener can be probed on its own.
pub fn apps(state: &AppState, shutdown: &CancellationToken) -> Apps {
    let readiness = readiness(state.db.clone(), shutdown.clone());
    Apps {
        client_api: client_api_router(readiness.clone()),
        admin: admin_router(readiness),
        errors: ErrorBodies {
            client_api: deskhaven_client_api::render_stack_error,
            admin: deskhaven_admin_api::render_stack_error,
        },
    }
}

/// `/api/*` (RustDesk client contract) joins here.
fn client_api_router(readiness: impl ReadinessCheck) -> Router {
    health_routes(readiness)
}

/// `/admin/api/*` joins here. The console serves every other path through
/// its fallback, so it must stay the only router with a fallback.
fn admin_router(readiness: impl ReadinessCheck) -> Router {
    health_routes(readiness).merge(deskhaven_web::router())
}

/// `/readyz`: the database answers a ping, and the process is not shutting
/// down. Migrations are applied by construction (requests are only served
/// after `migrate` succeeded). Settings load warnings do not count: the
/// service runs on safe fallbacks, and a restart would not fix a bad row.
/// The ping uses the shared pool, so a saturated pool (no connection within
/// [`READY_PROBE_TIMEOUT`]) also reports not ready.
pub fn readiness(
    db: DatabaseConnection,
    shutdown: CancellationToken,
) -> impl ReadinessCheck + Clone {
    move || probe(db.clone(), shutdown.clone())
}

async fn probe(db: DatabaseConnection, shutdown: CancellationToken) -> Result<(), NotReady> {
    if shutdown.is_cancelled() {
        return Err(NotReady::new("shutting down"));
    }
    match tokio::time::timeout(READY_PROBE_TIMEOUT, db.ping()).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => Err(NotReady::new(format!(
            "database ping failed: {}",
            log_safe::render_error(&err)
        ))),
        Err(_) => Err(NotReady::new("database ping timed out")),
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use super::*;

    const DRAIN: Duration = Duration::from_secs(8);

    fn with_public_url(url: &str) -> Bootstrap {
        Bootstrap {
            public_url: url.to_owned(),
            ..Bootstrap::default()
        }
    }

    #[test]
    fn hsts_only_for_https_public_url() {
        let https = http_config(&with_public_url("https://rd.example.com"), DRAIN);
        let upper = http_config(&with_public_url("HTTPS://rd.example.com"), DRAIN);
        let http = http_config(&with_public_url("http://rd.example.com"), DRAIN);
        let none = http_config(&Bootstrap::default(), DRAIN);

        assert_eq!(https.strict_transport_security.unwrap(), HSTS);
        assert!(upper.strict_transport_security.is_some());
        assert!(http.strict_transport_security.is_none());
        assert!(none.strict_transport_security.is_none());
    }

    #[test]
    fn admin_hsts_follows_admin_public_url() {
        let mut config = with_public_url("http://rd.example.com");
        config.listen.admin_public_url = "https://console.example.com".into();

        let http = http_config(&config, DRAIN);

        assert!(http.strict_transport_security.is_none());
        assert_eq!(http.admin_strict_transport_security.unwrap(), HSTS);
    }

    #[test]
    fn maps_proxies_csp_and_drain() {
        let mut config = Bootstrap::default();
        config.security.trusted_proxies = vec!["10.0.0.0/8".parse().unwrap()];

        let http = http_config(&config, DRAIN);

        assert!(http.trusted_proxies.is_trusted("10.1.2.3".parse().unwrap()));
        assert!(!http.trusted_proxies.is_trusted("11.0.0.1".parse().unwrap()));
        assert_eq!(http.drain_timeout, DRAIN);
        assert_eq!(
            http.content_security_policy.as_ref().unwrap(),
            deskhaven_web::CSP
        );
        assert_eq!(http.validate(), Ok(()));
    }

    #[test]
    fn listen_topology() {
        let client: SocketAddr = "127.0.0.1:21114".parse().unwrap();
        let admin: SocketAddr = "127.0.0.1:21113".parse().unwrap();
        let single = Listen {
            client_api: client,
            admin: Some(client),
            admin_public_url: "https://ignored.example".into(),
        };
        let split = Listen {
            client_api: client,
            admin: Some(admin),
            admin_public_url: String::new(),
        };

        let single = listen_config(&single);
        let split = listen_config(&split);

        assert_eq!(single.admin, None);
        assert_eq!(split.admin, Some(admin));
        assert_eq!(split.admin_public_url, None);
    }
}
