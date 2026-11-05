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

# Installs the channel and components declared in rust-toolchain.toml.
rustup toolchain install --profile minimal
rustc --version

# Extra tools via `cargo install --locked` (no caching until a prebuilt CI image exists).
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
