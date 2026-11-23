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

# Extra tools. On x86_64, cargo-deny, cargo-nextest and cargo-llvm-cov are their projects' prebuilt release
# binaries, each tarball's sha256 pinned in .github/ci-tools.sha256 (add a line when bumping
# a version); everything else, and every tool elsewhere, is `cargo install --locked`.
# A spec is name@version, optionally name@version/feature,feature (cargo-about needs `cli`).
TOOLS_SHA256="$(cd "$(dirname "$0")" && pwd)/ci-tools.sha256"

prebuilt_url() { # name version -> "url local-tarball-name binary-path-in-tarball", or nothing
  local triple=x86_64-unknown-linux-musl
  case "$1" in
    cargo-deny)
      echo "https://github.com/EmbarkStudios/cargo-deny/releases/download/$2/cargo-deny-$2-$triple.tar.gz" \
        "cargo-deny-$2-$triple.tar.gz" "cargo-deny-$2-$triple/cargo-deny"
      ;;
    cargo-nextest)
      echo "https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-$2/cargo-nextest-$2-$triple.tar.gz" \
        "cargo-nextest-$2-$triple.tar.gz" "cargo-nextest"
      ;;
    cargo-llvm-cov)
      echo "https://github.com/taiki-e/cargo-llvm-cov/releases/download/v$2/cargo-llvm-cov-$triple.tar.gz" \
        "cargo-llvm-cov-$2-$triple.tar.gz" "cargo-llvm-cov"
      ;;
  esac
}

install_prebuilt() { # name version url tarball member
  local bin_dir="${CARGO_HOME:-$HOME/.cargo}/bin" tmp
  # As cargo subcommands: `cargo-llvm-cov --version` alone is not a valid invocation.
  if [ -x "$bin_dir/$1" ] && cargo "${1#cargo-}" --version 2>/dev/null | grep -qF " $2"; then
    echo "$1 $2 already installed"
    return
  fi
  grep -qF "  $4" "$TOOLS_SHA256" || {
    echo "::error::$TOOLS_SHA256 has no entry for $4" >&2
    exit 1
  }
  tmp="$(mktemp -d)"
  curl -fsSL -o "$tmp/$4" "$3"
  (cd "$tmp" && grep -F "  $4" "$TOOLS_SHA256" | sha256sum -c -)
  tar -xzf "$tmp/$4" -C "$tmp" "$5"
  mkdir -p "$bin_dir"
  install -m 0755 "$tmp/$5" "$bin_dir/$1"
  rm -rf "$tmp"
  cargo "${1#cargo-}" --version
}

for spec in "$@"; do
  name="${spec%%@*}"
  version="${spec#*@}"
  features="${version#*/}"
  version="${version%%/*}"
  prebuilt=""
  if [ "$(uname -m)" = x86_64 ]; then prebuilt="$(prebuilt_url "$name" "$version")"; fi
  if [ -n "$prebuilt" ]; then
    # shellcheck disable=SC2086 # three words, none with spaces
    install_prebuilt "$name" "$version" $prebuilt
  elif [ "$features" = "$version" ] || [ -z "$features" ]; then
    cargo install --locked "$name" --version "$version"
  else
    cargo install --locked "$name" --version "$version" --features "$features"
  fi
done
