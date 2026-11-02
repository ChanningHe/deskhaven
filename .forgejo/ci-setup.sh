#!/usr/bin/env bash
# Shared CI bootstrap: system packages + the toolchain pinned in rust-toolchain.toml.
# Usage: .forgejo/ci-setup.sh [extra cargo tools, e.g. "cargo-deny@0.20.2"]
set -euo pipefail

export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq --no-install-recommends \
  pkg-config protobuf-compiler libsodium-dev libclang-dev >/dev/null

# Installs the channel and components declared in rust-toolchain.toml.
rustup toolchain install --profile minimal
rustc --version

# Extra tools via `cargo install --locked`; ~/.cargo/bin is cached by the workflow.
for spec in "$@"; do
  name="${spec%@*}"
  version="${spec#*@}"
  # Restored from the workflow cache: the cache key encodes the pinned version.
  if command -v "$name" >/dev/null; then
    continue
  fi
  if [ "$name" = "$version" ]; then
    cargo install --locked "$name"
  else
    cargo install --locked "$name" --version "$version"
  fi
done
