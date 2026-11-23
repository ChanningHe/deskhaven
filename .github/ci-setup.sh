#!/usr/bin/env bash
# Shared CI bootstrap: system packages + the toolchain pinned in rust-toolchain.toml.
# Usage: [CI_SKIP_APT=1] .github/ci-setup.sh [cargo tools as name@version ...]
set -euo pipefail

APT_ATTEMPTS=3
APT_RETRY_SLEEP_SECS=5

apt_update_with_retry() {
  local attempt
  for attempt in $(seq 1 "$APT_ATTEMPTS"); do
    if apt-get update -qq; then
      return 0
    fi
    echo "apt-get update failed (attempt $attempt/$APT_ATTEMPTS)" >&2
    sleep "$APT_RETRY_SLEEP_SECS"
  done
  return 1
}

if [ "${CI_SKIP_APT:-0}" != "1" ]; then
  export DEBIAN_FRONTEND=noninteractive
  apt_update_with_retry
  apt-get install -y -qq --no-install-recommends \
    pkg-config protobuf-compiler libsodium-dev libssl-dev >/dev/null
fi

# A persistent build directory (CARGO_TARGET_DIR on the self-hosted runner's cache volume,
# see ci.yml) starts over once it outgrows its share of the disk: stale artifacts of old
# dependency versions pile up there and cargo never removes them.
CI_TARGET_MAX_GB="${CI_TARGET_MAX_GB:-25}"
if [ -n "${CARGO_TARGET_DIR:-}" ] && [ -d "$CARGO_TARGET_DIR" ]; then
  used_mb="$(du -sm "$CARGO_TARGET_DIR" | cut -f1)"
  echo "build cache $CARGO_TARGET_DIR: ${used_mb} MB (limit ${CI_TARGET_MAX_GB} GB)"
  if [ "$used_mb" -ge $((CI_TARGET_MAX_GB * 1024)) ]; then
    rm -rf "$CARGO_TARGET_DIR"
    echo "build cache cleared"
  fi
fi

# Installs the channel and components declared in rust-toolchain.toml.
rustup toolchain install --profile minimal
rustc --version

# Extra tools via `cargo install --locked`; with a persistent CARGO_HOME an installed version
# is kept and the install is a no-op.
# A spec is name@version, optionally name@version/feature,feature (cargo-about needs `cli`).
for spec in "$@"; do
  name="${spec%%@*}"
  version="${spec#*@}"
  features="${version#*/}"
  version="${version%%/*}"
  if [ "$features" = "$version" ] || [ -z "$features" ]; then
    cargo install --locked "$name" --version "$version"
  else
    cargo install --locked "$name" --version "$version" --features "$features"
  fi
done
