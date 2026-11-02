//! Re-embed when the SPA is rebuilt; rust-embed cannot tell cargo about it.

use std::path::Path;

const DIST: &str = "../../web/dist";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // A missing path counts as changed on every build, which would recompile this
    // crate each time. Without a dist there is nothing to watch: `xtask dist`
    // drops the stale release artifact of this crate before it builds.
    if Path::new(DIST).exists() {
        println!("cargo:rerun-if-changed={DIST}");
    }
}
