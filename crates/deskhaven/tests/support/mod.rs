//! Harness for the end-to-end tests of `deskhaven serve`: the real binary in a temporary
//! working directory, its JSON logs captured, plain HTTP/1.1 requests.

// Each test binary uses a different subset.
#![allow(dead_code)]

use std::hash::{BuildHasher, RandomState};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::Value;

pub const STARTUP_LIMIT: Duration = Duration::from_secs(20);
/// The server's own deadline is 10s; allow for process teardown.
pub const SHUTDOWN_LIMIT: Duration = Duration::from_secs(12);
/// Startup gives up on an unreachable database after its 10s connect timeout.
pub const FAILURE_LIMIT: Duration = Duration::from_secs(30);
pub const READY_LIMIT: Duration = Duration::from_secs(5);
pub const POLL: Duration = Duration::from_millis(50);
pub const HTTP_TIMEOUT: Duration = Duration::from_secs(5);

/// base64 of [0x5a; 32]: easy to spot in a log.
pub fn planted_master_key() -> String {
    format!("{}Wlo=", "Wlpa".repeat(10))
}

pub fn workdir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("serve")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// JSON logs, client API on an OS-assigned loopback port, signal server
/// disabled (its fixed ports would collide between parallel tests);
/// `listen_extra` is appended to the `[listen]` table.
pub fn config(dir: &Path, listen_extra: &str) -> String {
    config_with_signal(dir, listen_extra, "enabled = false")
}

/// [`config`] with `signal` as the `[signal]` table.
pub fn config_with_signal(dir: &Path, listen_extra: &str, signal: &str) -> String {
    format!(
        "data_dir = \"{}\"\n\n[listen]\nclient_api = \"127.0.0.1:0\"\n{listen_extra}\n\n[signal]\n{signal}\n\n[log]\nformat = \"json\"\n",
        dir.join("data").display()
    )
}

/// A port that was free a moment ago.
pub fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Lowest port the kernel hands out for ephemeral sockets (listeners on port 0 and the source
/// ports of outgoing connections), from `ip_local_port_range`; the IANA default if unreadable.
fn ephemeral_floor() -> u16 {
    const IANA_FLOOR: u16 = 49152;
    std::fs::read_to_string("/proc/sys/net/ipv4/ip_local_port_range")
        .ok()
        .and_then(|range| range.split_whitespace().next()?.parse().ok())
        .unwrap_or(IANA_FLOOR)
}

/// First port of the signal port blocks, above the registered-service ports.
const SIGNAL_PORT_FLOOR: u16 = 10_000;
/// Ports from `base - 1` to `base + 2` belong to one signal server; blocks are this far apart.
const SIGNAL_BLOCK: u16 = 4;

/// A signal base port whose TCP port-1, port, port+2 and UDP port are free on all IPv4
/// interfaces (the server binds all interfaces).
///
/// Probing alone is racy: the probe releases the ports again, and an ephemeral socket of a
/// parallel test (a `:0` listener, a client's source port) can take one before the caller
/// binds it. So blocks are drawn from below the ephemeral range, where the kernel never
/// allocates on its own, and handed out once per process by a shared cursor that starts at
/// a per-process offset (concurrent test processes rarely overlap); the probe still skips
/// ports some other program holds.
pub fn free_signal_port() -> u16 {
    static CURSOR: AtomicU16 = AtomicU16::new(0);
    // Random per process, not pid-derived: CI jobs in separate containers have similar small
    // pids but may share the host network, and would otherwise walk the same blocks at once.
    static START: OnceLock<u32> = OnceLock::new();
    let any = |port: u16| SocketAddr::from(([0, 0, 0, 0], port));
    let blocks = (ephemeral_floor().saturating_sub(SIGNAL_PORT_FLOOR + 2) / SIGNAL_BLOCK).max(1);
    let seed = *START.get_or_init(|| {
        let random = RandomState::new().hash_one(std::process::id());
        u32::try_from(random % u64::from(u32::MAX)).unwrap()
    });
    let start = seed % u32::from(blocks);
    for _ in 0..blocks {
        let step = CURSOR.fetch_add(1, Ordering::Relaxed);
        let block = (start + u32::from(step)) % u32::from(blocks);
        let port = SIGNAL_PORT_FLOOR + 1 + u16::try_from(block).unwrap() * SIGNAL_BLOCK;
        let tcp_free = [port - 1, port, port + 2]
            .iter()
            .all(|p| TcpListener::bind(any(*p)).is_ok());
        if tcp_free && std::net::UdpSocket::bind(any(port)).is_ok() {
            return port;
        }
    }
    panic!("no free signal port block below the ephemeral range");
}

/// A free signal port block whose WebSocket port (`port + 2`) the caller then holds. Another
/// process can take a probed port before the test binds it, so a lost race moves on to the
/// next block instead of failing the test.
pub fn signal_port_with_ws_taken() -> (u16, TcpListener) {
    const ATTEMPTS: usize = 32;
    for _ in 0..ATTEMPTS {
        let port = free_signal_port();
        match TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], port + 2))) {
            Ok(taken) => return (port, taken),
            Err(err) if err.kind() == std::io::ErrorKind::AddrInUse => continue,
            Err(err) => panic!("binding the WebSocket port {}: {err}", port + 2),
        }
    }
    panic!("every signal port block was taken by someone else");
}

pub struct Server {
    child: Child,
    output: Arc<Mutex<String>>,
    readers: Vec<JoinHandle<()>>,
}

pub fn capture(stream: impl Read + Send + 'static, output: &Arc<Mutex<String>>) -> JoinHandle<()> {
    let output = Arc::clone(output);
    std::thread::spawn(move || {
        for line in BufReader::new(stream).lines() {
            let Ok(line) = line else { break };
            let mut all = output.lock().unwrap();
            all.push_str(&line);
            all.push('\n');
        }
    })
}

pub fn start(dir: &Path, config: &str, env: &[(&str, &str)]) -> Server {
    std::fs::write(dir.join("deskhaven.toml"), config).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_deskhaven"))
        .args(["--config", "deskhaven.toml", "serve"])
        .env_clear()
        .envs(env.iter().copied())
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let output = Arc::new(Mutex::new(String::new()));
    let readers = vec![
        capture(child.stdout.take().unwrap(), &output),
        capture(child.stderr.take().unwrap(), &output),
    ];
    Server {
        child,
        output,
        readers,
    }
}

impl Server {
    pub fn snapshot(&self) -> String {
        self.output.lock().unwrap().clone()
    }

    /// The first JSON log event whose `fields` satisfy `pred`.
    pub fn wait_for_event(&mut self, pred: impl Fn(&Value) -> bool) -> Value {
        let started = Instant::now();
        loop {
            let found = self
                .snapshot()
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .find(|event| pred(&event["fields"]));
            if let Some(event) = found {
                return event["fields"].clone();
            }
            if let Some(status) = self.child.try_wait().unwrap() {
                panic!("server exited ({status}) early:\n{}", self.snapshot());
            }
            assert!(
                started.elapsed() < STARTUP_LIMIT,
                "timed out waiting for a log event:\n{}",
                self.snapshot()
            );
            std::thread::sleep(POLL);
        }
    }

    /// The bound address of `listener`, once requests are served (listeners
    /// bind before the database opens; until `ready` requests only queue).
    pub fn listening(&mut self, listener: &str) -> SocketAddr {
        let fields =
            self.wait_for_event(|f| f["message"] == "listening" && f["listener"] == listener);
        self.wait_for_event(|f| f["message"] == "ready");
        fields["addr"].as_str().unwrap().parse().unwrap()
    }

    pub fn public_key(&mut self) -> String {
        let fields = self.wait_for_event(|f| f["message"] == "hbbs key pair loaded");
        fields["public_key"].as_str().unwrap().to_owned()
    }

    pub fn wait_exit(&mut self, limit: Duration) -> (ExitStatus, Duration) {
        let started = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return (status, started.elapsed());
            }
            assert!(
                started.elapsed() < limit,
                "server did not exit within {limit:?}:\n{}",
                self.snapshot()
            );
            std::thread::sleep(POLL);
        }
    }

    pub fn terminate(&mut self) -> (ExitStatus, Duration) {
        let pid = rustix::process::Pid::from_raw(i32::try_from(self.child.id()).unwrap()).unwrap();
        rustix::process::kill_process(pid, rustix::process::Signal::TERM).unwrap();
        self.wait_exit(SHUTDOWN_LIMIT)
    }

    /// Everything the process wrote; call after it exited.
    pub fn logs(&mut self) -> String {
        for reader in self.readers.drain(..) {
            reader.join().unwrap();
        }
        self.snapshot()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// Status code and the raw response (head + body).
pub fn get(addr: SocketAddr, path: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream.set_read_timeout(Some(HTTP_TIMEOUT)).unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {addr}\r\nAccept: */*\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let status = response
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("bad response: {response}"));
    (status, response)
}

pub fn wait_ready(addr: SocketAddr) {
    let started = Instant::now();
    while get(addr, "/readyz").0 != 200 {
        assert!(started.elapsed() < READY_LIMIT, "/readyz never became 200");
        std::thread::sleep(POLL);
    }
}

pub fn read_trimmed(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap().trim().to_owned()
}

/// Forms of the hbbs private key that must never be logged: the file
/// (base64 of seed + public key) and the base64 of the 32-byte seed alone.
pub fn hbbs_secrets(dir: &Path) -> [String; 2] {
    use base64_022::Engine as _;
    use base64_022::engine::general_purpose::STANDARD;

    let file = read_trimmed(&dir.join("data/id_ed25519"));
    let pair = STANDARD.decode(&file).unwrap();
    let seed = STANDARD.encode(&pair[..32]);
    [file, seed]
}

pub fn assert_no_hbbs_secret(dir: &Path, logs: &str) {
    for secret in hbbs_secrets(dir) {
        assert!(!logs.contains(&secret), "hbbs secret key leaked");
    }
}

pub fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

pub fn side_file(db: &Path, suffix: &str) -> PathBuf {
    let mut name = db.as_os_str().to_owned();
    name.push(suffix);
    name.into()
}

/// Size and mtime of the database file and its WAL side files.
pub fn fingerprint(db: &Path) -> Vec<Option<(u64, std::time::SystemTime)>> {
    ["", "-wal", "-shm"]
        .iter()
        .map(|suffix| {
            let meta = std::fs::metadata(side_file(db, suffix)).ok()?;
            Some((meta.len(), meta.modified().unwrap()))
        })
        .collect()
}

pub fn config_check(dir: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_deskhaven"))
        .args(["--config", "deskhaven.toml", "config", "check"])
        .env_clear()
        .current_dir(dir)
        .output()
        .unwrap()
}
