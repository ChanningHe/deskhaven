//! Rendezvous messages through the signal server's own handlers (`deskhaven_signal::fuzzing`).
//!
//! Input: the first byte picks the transport. Even: the rest is one UDP datagram. Odd: the rest
//! is the frames of one main-port TCP connection, each prefixed by its length (u16, little
//! endian; a short tail is the last frame). The server is kept across inputs (state-dependent
//! paths) and replaced every `RESET_EVERY` inputs to bound memory.
#![no_main]

use std::sync::{Mutex, OnceLock};

use deskhaven_signal::fuzzing::Harness;
use libfuzzer_sys::fuzz_target;
use tokio::runtime::Runtime;

const RESET_EVERY: u32 = 10_000;

struct State {
    runtime: Runtime,
    harness: Harness,
    runs: u32,
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let harness = runtime.block_on(Harness::new()).expect("harness");
        Mutex::new(State {
            runtime,
            harness,
            runs: 0,
        })
    })
}

fn frames(mut data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    while !data.is_empty() {
        if data.len() < 2 {
            out.push(data);
            break;
        }
        let len = usize::from(u16::from_le_bytes([data[0], data[1]]));
        let rest = &data[2..];
        let (frame, tail) = rest.split_at(len.min(rest.len()));
        out.push(frame);
        data = tail;
    }
    out
}

fuzz_target!(|data: &[u8]| {
    let Some((&mode, rest)) = data.split_first() else {
        return;
    };
    let mut guard = state().lock().unwrap_or_else(|e| e.into_inner());
    let State {
        runtime,
        harness,
        runs,
    } = &mut *guard;
    *runs += 1;
    runtime.block_on(async {
        if *runs % RESET_EVERY == 0 {
            harness.reset().await.expect("reset");
        }
        if mode % 2 == 0 {
            harness.udp(rest).await;
        } else {
            harness.tcp(frames(rest)).await;
        }
    });
});
