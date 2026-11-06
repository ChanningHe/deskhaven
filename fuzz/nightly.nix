# Nightly Rust and cargo-fuzz for the fuzz targets; the workspace itself stays on
# rust-toolchain.toml. Same nixpkgs and rust-overlay revisions as flake.lock.
#   nix develop --impure -f fuzz/nightly.nix --command cargo fuzz run --fuzz-dir fuzz <target>
let
  lock = builtins.fromJSON (builtins.readFile ../flake.lock);
  fetch =
    name:
    let
      node = lock.nodes.${lock.nodes.root.inputs.${name}}.locked;
    in
    builtins.fetchTarball {
      url = "https://github.com/${node.owner}/${node.repo}/archive/${node.rev}.tar.gz";
      sha256 = node.narHash;
    };
  pkgs = import (fetch "nixpkgs") {
    overlays = [ (import (fetch "rust-overlay")) ];
  };
in
pkgs.mkShell {
  packages = with pkgs; [
    # Pinned: the 2026-11-01 nightly fails on ldap3 ("needs async drop" query cycle, E0391).
    (rust-bin.nightly."2026-09-15".minimal.override { extensions = [ "rust-src" ]; })
    cargo-fuzz
    pkg-config
    protobuf
    libsodium
    openssl
    sqlite
  ];
  SODIUM_USE_PKG_CONFIG = "1";
}
