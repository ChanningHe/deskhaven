//! Re-embed when the SPA is rebuilt; rust-embed cannot tell cargo about it.

use std::path::Path;

const DIST: &str = "../../web/dist";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // Watched even when missing: cargo treats a missing path as changed, so the next build after
    // `web/dist` appears (or changes) re-embeds it.
    println!("cargo:rerun-if-changed={DIST}");
    if !Path::new(DIST).exists() {
        println!("cargo:warning=web/dist not found; console will serve the not-built page");
    }
}
