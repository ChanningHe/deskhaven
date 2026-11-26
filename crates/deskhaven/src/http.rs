use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::http::HeaderValue;
use deskhaven_admin_api::{AdminRoutes, TrustedOrigins};
use deskhaven_config::Cidr;
use deskhaven_config::bootstrap::{Bootstrap, Listen};
use deskhaven_core::state::AppState;
use deskhaven_core::web_client::ws_origin;
use deskhaven_http::{
    Apps, ErrorBodies, HttpConfig, ListenConfig, NotReady, ReadinessCheck, TrustedProxies,
    health_routes,
};
use sea_orm::DatabaseConnection;
use tokio_util::sync::CancellationToken;

use crate::db_ping::{CachedPing, PING_TTL};
use crate::signal_server::SignalStatus;

/// HSTS for https deployments: one year, without `includeSubDomains`
/// (other hosts under the domain are not ours to pin).
pub const HSTS: &str = "max-age=31536000";
pub const READY_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const HTTPS_PREFIX: &str = "https://";

pub fn http_config(config: &Bootstrap, drain_timeout: Duration) -> HttpConfig {
    HttpConfig {
        trusted_proxies: TrustedProxies::new(config.security.trusted_proxies.iter().map(Cidr::net)),
        drain_timeout,
        content_security_policy: Some(console_csp(config)),
        strict_transport_security: hsts_for(&config.public_url),
        admin_strict_transport_security: hsts_for(&config.listen.admin_public_url),
        ..HttpConfig::default()
    }
}

/// Plus the web client relay's WebSocket origin when the console is served from another origin.
fn console_csp(config: &Bootstrap) -> HeaderValue {
    let fixed = HeaderValue::from_static(deskhaven_web::CSP);
    let Some(relay) = ws_origin(&config.public_url) else {
        return fixed;
    };
    let console_url = match config.listen.admin_addr() {
        Some(_) => &config.listen.admin_public_url,
        None => &config.public_url,
    };
    if ws_origin(console_url).as_ref() == Some(&relay) {
        return fixed;
    }
    deskhaven_web::csp_connecting_to(&relay)
        .and_then(|csp| HeaderValue::from_str(&csp).ok())
        .unwrap_or(fixed)
}

fn hsts_for(url: &str) -> Option<HeaderValue> {
    url.get(..HTTPS_PREFIX.len())
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case(HTTPS_PREFIX))
        .then(|| HeaderValue::from_static(HSTS))
}

pub fn trusted_origins(config: &Bootstrap) -> TrustedOrigins {
    TrustedOrigins::from_urls([
        config.public_url.as_str(),
        config.listen.admin_public_url.as_str(),
    ])
}

pub fn listen_config(listen: &Listen) -> ListenConfig {
    ListenConfig {
        client_api: listen.client_api,
        admin: listen.admin_addr(),
        admin_public_url: Some(listen.admin_public_url.clone()).filter(|url| !url.is_empty()),
    }
}

/// Health routes are on both, so a split admin listener can be probed on its own.
pub fn apps(
    state: Arc<AppState>,
    admin_routes: AdminRoutes,
    shutdown: &CancellationToken,
    signal: Option<SignalStatus>,
) -> Apps {
    let readiness = readiness(state.db.clone(), shutdown.clone(), signal);
    Apps {
        admin: admin_router(
            admin_routes.into_router(Arc::clone(&state)),
            readiness.clone(),
        ),
        client_api: client_api_router(state, readiness),
        errors: ErrorBodies {
            client_api: deskhaven_client_api::render_stack_error,
            admin: deskhaven_admin_api::render_stack_error,
        },
    }
}

fn client_api_router(state: Arc<AppState>, readiness: impl ReadinessCheck) -> Router {
    health_routes(readiness).merge(deskhaven_client_api::router().with_state(state))
}

/// The console serves every other path through its fallback, so it must stay the only router
/// with a fallback.
fn admin_router(admin_api: Router, readiness: impl ReadinessCheck) -> Router {
    health_routes(readiness)
        .merge(admin_api)
        .merge(deskhaven_client_api::version_router())
        .merge(deskhaven_web::router())
}

/// `/readyz` also requires the signal server: a deployment without rendezvous cannot connect
/// anyone. Settings load warnings do not count (a restart would not fix a bad row). The ping
/// uses the shared pool, so a saturated pool reports not ready; one ping answers every probe
/// for [`PING_TTL`] (the route is public).
pub fn readiness(
    db: DatabaseConnection,
    shutdown: CancellationToken,
    signal: Option<SignalStatus>,
) -> impl ReadinessCheck + Clone {
    let ping = CachedPing::new(db, PING_TTL, READY_PROBE_TIMEOUT);
    move || probe(ping.clone(), shutdown.clone(), signal.clone())
}

async fn probe(
    ping: CachedPing,
    shutdown: CancellationToken,
    signal: Option<SignalStatus>,
) -> Result<(), NotReady> {
    if shutdown.is_cancelled() {
        return Err(NotReady::new("shutting down"));
    }
    signal_ready(signal.as_ref())?;
    ping.ping().await.map_err(NotReady::new)
}

pub(crate) fn signal_ready(signal: Option<&SignalStatus>) -> Result<(), NotReady> {
    match signal {
        Some(status) if !status.is_serving() => Err(NotReady::new("signal server not serving")),
        _ => Ok(()),
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

    fn csp_of(config: &Bootstrap) -> String {
        let http = http_config(config, DRAIN);
        let csp = http.content_security_policy.unwrap();
        csp.to_str().unwrap().to_owned()
    }

    #[test]
    fn the_console_csp_admits_the_relay_only_on_another_origin() {
        let single = with_public_url("https://rd.example.com");
        let mut split = with_public_url("https://rd.example.com");
        split.listen.admin = Some("127.0.0.1:21113".parse().unwrap());
        split.listen.admin_public_url = "https://console.example.com".into();
        let mut split_same_host = split.clone();
        split_same_host.listen.admin_public_url = "https://RD.example.com/".into();
        let mut split_without_public_url = split.clone();
        split_without_public_url.public_url = String::new();

        assert_eq!(csp_of(&single), deskhaven_web::CSP);
        assert_eq!(csp_of(&split_same_host), deskhaven_web::CSP);
        assert_eq!(csp_of(&split_without_public_url), deskhaven_web::CSP);
        assert_eq!(
            csp_of(&split),
            deskhaven_web::CSP.replace(
                "connect-src 'self';",
                "connect-src 'self' wss://rd.example.com;"
            )
        );
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
