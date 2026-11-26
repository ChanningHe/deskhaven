//! Its own test binary: it lowers the process's open-file limit.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use hbb_common::tokio::{self, time::Duration};
use rustix::process::{Resource, getrlimit, setrlimit};
use support::{TcpClient, TestServer, read_offer};

/// Longer than one accept back-off, so the server certainly hits EMFILE meanwhile.
const STARVED: Duration = Duration::from_millis(600);

fn max_fd() -> u64 {
    std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse::<u64>().ok())
        .max()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_connection_queued_while_descriptors_ran_out_is_served_afterwards() {
    // No descriptor left but one, which the client's socket takes.
    let server = TestServer::start().await;
    let limit = getrlimit(Resource::Nofile);
    let mut starved = limit;
    starved.current = Some(max_fd() + 1);
    setrlimit(Resource::Nofile, starved).unwrap();
    let mut fillers = Vec::new();
    while let Ok(file) = std::fs::File::open("/dev/null") {
        fillers.push(file);
    }
    fillers.pop();

    // The server's accept fails with EMFILE while the client waits in the backlog.
    let mut client = TcpClient::connect(server.main_addr()).await;
    tokio::time::sleep(STARVED).await;
    setrlimit(Resource::Nofile, limit).unwrap();
    drop(fillers);

    read_offer(&mut client).await;
    server.stop().await;
}
