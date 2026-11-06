//! Client API requests through the production router and middleware (`deskhaven-contract`'s
//! in-process app: migrated SQLite, default settings, the real extractors and handlers).
//!
//! Input: byte 0 picks the endpoint, byte 1 bit 0 adds the seeded client session's bearer
//! token; the rest is the JSON body (bodies are lossily UTF-8 decoded), or the query string of a
//! GET endpoint (percent-encoded except `=` and `&`). Seeded: a client session, a registered
//! device and a personal address book, so `{guid}` paths and id/uuid checks can pass. A panic
//! anywhere (libFuzzer aborts on any panic, caught or not) or a 500 reply is a finding.
#![no_main]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::OnceLock;

use deskhaven_contract::app::TestApp;
use deskhaven_contract::fixture::{Body, Request};
use deskhaven_contract::seeds::{SeedOutputs, Seeds};
use libfuzzer_sys::fuzz_target;
use tokio::runtime::Runtime;

/// (method, path); `{guid}` is the seeded personal book. GET endpoints take a query.
const ENDPOINTS: &[(&str, &str)] = &[
    ("POST", "/api/heartbeat"),
    ("POST", "/api/sysinfo"),
    ("POST", "/api/audit/conn"),
    ("POST", "/api/audit/file"),
    ("POST", "/api/audit/alarm"),
    ("POST", "/api/login"),
    ("POST", "/api/currentUser"),
    ("POST", "/api/logout"),
    ("POST", "/api/devices/cli"),
    ("POST", "/api/devices/deploy"),
    ("POST", "/api/oidc/auth"),
    ("GET", "/api/oidc/auth-query"),
    ("GET", "/api/oidc/callback"),
    ("GET", "/api/login-options"),
    ("GET", "/api/users"),
    ("GET", "/api/peers"),
    ("GET", "/api/device-group/accessible"),
    ("GET", "/api/ab"),
    ("POST", "/api/ab"),
    ("POST", "/api/ab/get"),
    ("POST", "/api/ab/personal"),
    ("POST", "/api/ab/settings"),
    ("POST", "/api/ab/shared/profiles"),
    ("POST", "/api/ab/peers"),
    ("POST", "/api/ab/peer/add/{guid}"),
    ("PUT", "/api/ab/peer/update/{guid}"),
    ("DELETE", "/api/ab/peer/{guid}"),
    ("POST", "/api/ab/tags/{guid}"),
    ("POST", "/api/ab/tag/add/{guid}"),
    ("PUT", "/api/ab/tag/rename/{guid}"),
    ("PUT", "/api/ab/tag/update/{guid}"),
    ("DELETE", "/api/ab/tag/{guid}"),
];

struct Fixture {
    runtime: Runtime,
    app: TestApp,
    seeds: SeedOutputs,
}

fn fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let (app, seeds) = runtime.block_on(async {
            let app = TestApp::start().await.expect("app");
            let names = ["client-session", "registered-device", "ab-personal"].map(String::from);
            let seeds = Seeds::builtin()
                .apply(&names, &app.state)
                .await
                .expect("seeds");
            (app, seeds)
        });
        Fixture {
            runtime,
            app,
            seeds,
        }
    })
}

fn query(bytes: &[u8]) -> String {
    let mut out = String::new();
    for &b in bytes {
        if b.is_ascii_alphanumeric() || b == b'=' || b == b'&' {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

fn request(f: &Fixture, data: &[u8]) -> Option<Request> {
    let (&pick, rest) = data.split_first()?;
    let (&flags, payload) = rest.split_first()?;
    let (method, path) = ENDPOINTS[usize::from(pick) % ENDPOINTS.len()];
    let guid = f.seeds.get("personal_guid").map_or("", String::as_str);
    let mut path = path.replace("{guid}", guid);
    let mut headers = BTreeMap::new();
    headers.insert("content-type".to_owned(), "application/json".to_owned());
    if flags & 1 == 1 {
        let token = f.seeds.get("session_token").map_or("", String::as_str);
        headers.insert("authorization".to_owned(), format!("Bearer {token}"));
    }
    let body = if method == "GET" {
        path = format!("{path}?{}", query(payload));
        Body::Empty
    } else {
        Body::Text(String::from_utf8_lossy(payload).into_owned())
    };
    Some(Request {
        method: method.to_owned(),
        path,
        headers,
        body,
        peer: None,
    })
}

fuzz_target!(|data: &[u8]| {
    let f = fixture();
    let Some(req) = request(f, data) else {
        return;
    };
    let Ok(reply) = f.runtime.block_on(f.app.send(&req)) else {
        return;
    };
    assert_ne!(reply.status, 500, "{} {}: {:?}", req.method, req.path, reply);
});
