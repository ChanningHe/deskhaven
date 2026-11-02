//! End-to-end tests of `deskhaven serve`: the real binary with a temporary
//! data_dir and SQLite, JSON logs on stderr, plain HTTP/1.1 over TCP.
#![cfg(unix)]
// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]

mod support;

use std::net::TcpListener;
use std::path::Path;

use serde_json::Value;
use support::*;

#[test]
fn serves_health_and_shuts_down_cleanly_on_sigterm() {
    let dir = workdir("lifecycle");
    let master = planted_master_key();
    let mut server = start(
        &dir,
        &config(&dir, ""),
        &[("DESKHAVEN_SECURITY__MASTER_KEY", &master)],
    );

    let addr = server.listening("client_api");
    assert_eq!(get(addr, "/healthz").0, 200);
    wait_ready(addr);
    let (status, took) = server.terminate();

    let logs = server.logs();
    assert_eq!(status.code(), Some(0), "{logs}");
    assert!(took < SHUTDOWN_LIMIT, "{took:?}");
    assert!(logs.contains("shutdown complete"), "{logs}");
    assert!(!logs.contains(&master), "master key leaked:\n{logs}");
    assert_no_hbbs_secret(&dir, &logs);
    assert!(
        !dir.join("data/master.key").exists(),
        "a configured master key must not be written to disk"
    );
}

/// Review M3: umask 0077, so the database (and any side file) is owner-only.
#[test]
fn database_files_are_owner_only() {
    let dir = workdir("umask");
    let mut server = start(&dir, &config(&dir, ""), &[]);
    let addr = server.listening("client_api");
    wait_ready(addr);

    let db = dir.join("data/deskhaven.db");
    for path in [db.clone(), side_file(&db, "-wal"), side_file(&db, "-shm")] {
        if path.exists() {
            assert_eq!(mode_of(&path), 0o600, "{}", path.display());
        }
    }
    assert_eq!(mode_of(&db), 0o600);
    assert_eq!(mode_of(&dir.join("data")), 0o700);
    let (status, _) = server.terminate();
    assert_eq!(status.code(), Some(0), "{}", server.logs());
}

/// Review M1: one process per data_dir (advisory lock).
#[test]
fn second_instance_on_the_same_data_dir_is_refused() {
    let dir = workdir("second-instance");
    let mut first = start(&dir, &config(&dir, ""), &[]);
    first.listening("client_api");

    let second_dir = dir.join("second");
    std::fs::create_dir_all(&second_dir).unwrap();
    let mut second = start(&second_dir, &config(&dir, ""), &[]);
    let (status, _) = second.wait_exit(SHUTDOWN_LIMIT);

    let logs = second.logs();
    assert!(!status.success(), "{logs}");
    assert!(
        logs.contains("another deskhaven instance is using"),
        "{logs}"
    );
    let (status, _) = first.terminate();
    assert_eq!(status.code(), Some(0), "{}", first.logs());
}

/// Review M1: listeners are bound before the database is opened, so an
/// instance that cannot bind never creates or migrates a database.
#[test]
fn occupied_port_fails_before_the_database_is_touched() {
    let dir = workdir("port-taken");
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let config = config(&dir, "").replace(
        "client_api = \"127.0.0.1:0\"",
        &format!("client_api = \"{}\"", taken.local_addr().unwrap()),
    );
    let mut server = start(&dir, &config, &[]);

    let (status, _) = server.wait_exit(SHUTDOWN_LIMIT);

    let logs = server.logs();
    assert!(!status.success(), "{logs}");
    assert!(logs.contains("cannot bind client_api listener"), "{logs}");
    assert!(!dir.join("data/deskhaven.db").exists(), "{logs}");
}

/// Review H1: `config check` on a stopped server's WAL database creates no
/// `-wal`/`-shm` files and changes nothing.
#[test]
fn config_check_leaves_a_stopped_server_database_untouched() {
    let dir = workdir("check-after-stop");
    let mut server = start(&dir, &config(&dir, ""), &[]);
    server.listening("client_api");
    let (status, _) = server.terminate();
    assert_eq!(status.code(), Some(0), "{}", server.logs());
    let db = dir.join("data/deskhaven.db");
    let before = fingerprint(&db);
    assert_eq!(before[1..], [None, None], "server left WAL side files");

    let out = config_check(&dir);

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fingerprint(&db),
        before,
        "config check changed the database files"
    );
}

#[test]
fn second_start_reuses_key_material() {
    let dir = workdir("restart");
    let mut first = start(&dir, &config(&dir, ""), &[]);
    let first_key = first.public_key();
    first.listening("client_api");
    let master = read_trimmed(&dir.join("data/master.key"));
    let (status, _) = first.terminate();
    let first_logs = first.logs();
    assert_eq!(status.code(), Some(0), "{first_logs}");

    let mut second = start(&dir, &config(&dir, ""), &[]);
    let second_key = second.public_key();
    second.listening("client_api");
    let (status, _) = second.terminate();
    let second_logs = second.logs();

    assert_eq!(status.code(), Some(0), "{second_logs}");
    assert_eq!(first_key, second_key);
    assert_eq!(read_trimmed(&dir.join("data/master.key")), master);
    for logs in [first_logs, second_logs] {
        assert!(!logs.contains(&master), "master key leaked:\n{logs}");
        assert_no_hbbs_secret(&dir, &logs);
    }
}

/// M3-02 (M0-09 deferred): the first start logs a one-time setup link (token in the
/// fragment, no password); the next start issues none while that one is live.
#[test]
fn first_start_logs_a_setup_link_once() {
    let dir = workdir("setup-link");
    let is_link = |f: &Value| {
        f["message"]
            .as_str()
            .unwrap_or("")
            .starts_with("first-run setup:")
    };
    let mut first = start(&dir, &config(&dir, ""), &[]);
    let fields = first.wait_for_event(is_link);
    first.listening("client_api");
    let (status, _) = first.terminate();
    let first_logs = first.logs();
    assert_eq!(status.code(), Some(0), "{first_logs}");

    let mut second = start(&dir, &config(&dir, ""), &[]);
    let pending = second.wait_for_event(|f| {
        f["message"]
            .as_str()
            .unwrap_or("")
            .contains("setup pending")
    });
    second.listening("client_api");
    let (status, _) = second.terminate();
    let second_logs = second.logs();

    assert_eq!(status.code(), Some(0), "{second_logs}");
    let link = fields["link"].as_str().unwrap();
    let token = link.strip_prefix("/setup#token=").expect(link);
    assert_eq!(token.len(), 43, "{link}");
    assert!(
        !first_logs.to_lowercase().contains("password\":"),
        "{first_logs}"
    );
    assert!(pending["expires_at"].is_string(), "{pending}");
    assert!(!second_logs.contains(token), "{second_logs}");
}

#[test]
fn split_admin_listener_serves_health_and_client_root_redirects() {
    let dir = workdir("split");
    let admin_port = free_port();
    let listen = format!(
        "admin = \"127.0.0.1:{admin_port}\"\nadmin_public_url = \"https://console.example.test/\""
    );
    let mut server = start(&dir, &config(&dir, &listen), &[]);

    let client = server.listening("client_api");
    let admin = server.listening("admin");
    assert_eq!(admin.port(), admin_port);
    let (status, admin_health) = get(admin, "/healthz");
    assert_eq!(status, 200);
    wait_ready(admin);
    let (status, client_health) = get(client, "/healthz");
    assert_eq!(status, 200);
    // HSTS per listener: admin follows admin_public_url (https), client_api
    // follows public_url (unset).
    assert!(
        header(&admin_health, "strict-transport-security").is_some(),
        "{admin_health}"
    );
    assert_eq!(
        header(&client_health, "strict-transport-security"),
        None,
        "{client_health}"
    );
    let (status, response) = get(client, "/");
    assert_eq!(status, 302, "{response}");
    assert!(
        response
            .to_ascii_lowercase()
            .contains("location: https://console.example.test/"),
        "{response}"
    );
    let (status, _) = server.terminate();
    assert_eq!(status.code(), Some(0), "{}", server.logs());
}

/// The header value for `name` (case-insensitive) in a raw response head.
fn header<'a>(response: &'a str, name: &str) -> Option<&'a str> {
    let head = response.split("\r\n\r\n").next()?;
    head.lines().skip(1).find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then_some(value.trim())
    })
}

/// The `<title>` line of the built `web/dist/index.html`.
fn index_html_title() -> String {
    let index = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/dist/index.html");
    let html = std::fs::read_to_string(index).unwrap();
    let start = html.find("<title>").unwrap();
    let end = html[start..].find("</title>").unwrap() + start;
    html[start..end].to_owned()
}

#[test]
fn single_listener_serves_the_console_with_csp_and_keeps_api_404s() {
    let dir = workdir("console");
    let mut server = start(&dir, &config(&dir, ""), &[]);
    let addr = server.listening("client_api");

    // index.html with a built web/dist, the 503 "not built" page without one.
    // Local runs usually lack web/dist; CI builds it before testing (M4-06),
    // so both branches are exercised there and here.
    let (expected_status, marker) = if deskhaven_web::is_built() {
        (200, index_html_title())
    } else {
        (503, "web console not built".to_owned())
    };
    for path in ["/", "/devices"] {
        let (status, response) = get(addr, path);
        assert_eq!(status, expected_status, "{path}: {response}");
        assert!(response.contains(&marker), "{path}: {response}");
        assert_eq!(
            header(&response, "content-security-policy"),
            Some(deskhaven_web::CSP),
            "{path}: {response}"
        );
    }
    let (status, response) = get(addr, "/api/unknown");
    assert_eq!(status, 404, "{response}");
    let body = response.split("\r\n\r\n").nth(1).unwrap_or_default();
    let json: Value = serde_json::from_str(body).unwrap_or_else(|_| panic!("{response}"));
    assert!(json["error"].is_string(), "{response}");

    let (status, _) = server.terminate();
    assert_eq!(status.code(), Some(0), "{}", server.logs());
}

#[test]
fn unreachable_postgres_fails_startup_without_leaking_the_url() {
    let dir = workdir("postgres-down");
    let password = "pw-Hunter2-secret";
    let url = format!("postgres://deskhaven:{password}@127.0.0.1:1/deskhaven");
    let mut server = start(
        &dir,
        &config(&dir, ""),
        &[("DESKHAVEN_DATABASE__URL", &url)],
    );

    let (status, _) = server.wait_exit(FAILURE_LIMIT);

    let logs = server.logs();
    assert!(!status.success(), "{logs}");
    assert!(logs.contains("failed to connect to the database"), "{logs}");
    assert!(!logs.contains(password), "password leaked:\n{logs}");
    assert!(!logs.contains("postgres://"), "URL leaked:\n{logs}");
    // Listeners bind first (review M1); serving never starts.
    assert!(!logs.contains("\"ready\""), "{logs}");
}

#[test]
fn invalid_pin_fails_startup_naming_only_the_key_and_writes_nothing() {
    let dir = workdir("bad-pin");
    let value = "forty-two-SECRETISH";
    let mut server = start(
        &dir,
        &config(&dir, ""),
        &[("DESKHAVEN_SETTINGS__AUTH__SESSION_TTL", value)],
    );

    let (status, _) = server.wait_exit(SHUTDOWN_LIMIT);

    let logs = server.logs();
    assert!(!status.success(), "{logs}");
    assert!(logs.contains("auth.session_ttl"), "{logs}");
    assert!(!logs.contains("SECRETISH"), "pin value echoed:\n{logs}");
    assert!(
        !dir.join("data").exists(),
        "a rejected configuration must not create data_dir"
    );
}
