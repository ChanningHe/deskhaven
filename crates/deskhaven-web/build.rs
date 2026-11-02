//! Re-embed when the SPA is rebuilt; rust-embed cannot tell cargo about it.

use std::path::Path;

const DIST: &str = "../../web/dist";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // Always watched, also when missing: cargo treats a missing path as changed, so
    // this crate is recompiled on every build until `web/dist` exists, and once it
    // appears (or any file in it changes) the next build re-embeds it. That is what
    // keeps a hand-built `pnpm -C web build && cargo build --release` from shipping
    // a stale embed; `xtask dist` is the documented path but not required.
    println!("cargo:rerun-if-changed={DIST}");
    if !Path::new(DIST).exists() {
        println!("cargo:warning=web/dist not found; console will serve the not-built page");
    }
}
