#!/usr/bin/env bash
# shellcheck source-path=SCRIPTDIR
# Assembles the third-party notice of the relay image (hbbr): the Rust crates of the pinned
# upstream rustdesk-server release, from ITS lockfile. The upstream source is cloned at the
# tag and commit pinned in deploy/docker/relay.Dockerfile and is not modified.
#
# Usage: .github/release/notices-relay.sh OUTPUT_FILE
# Needs: cargo-about, git and network access.
set -euo pipefail

output="$(realpath -m "${1:?usage: notices-relay.sh OUTPUT_FILE}")"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
release_dir="$root/.github/release"
dockerfile="$root/deploy/docker/relay.Dockerfile"

# shellcheck source=notices-lib.sh
. "$release_dir/notices-lib.sh"

arg() { sed -nE "s/^ARG $1=(.*)$/\1/p" "$dockerfile" | head -n 1; }
repo="$(arg UPSTREAM_REPO)"
tag="$(arg UPSTREAM_TAG)"
commit="$(arg UPSTREAM_COMMIT)"
[ -n "$repo" ] && [ -n "$tag" ] && [ -n "$commit" ] || {
  echo "notices-relay.sh: UPSTREAM_REPO/TAG/COMMIT not found in $dockerfile" >&2
  exit 1
}

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
git -c advice.detachedHead=false clone -q --depth 1 --branch "$tag" --recurse-submodules --shallow-submodules "$repo" "$work/upstream"
[ "$(git -C "$work/upstream" rev-parse HEAD)" = "$commit" ] || {
  echo "notices-relay.sh: $tag no longer points to $commit" >&2
  exit 1
}

cd "$work/upstream"
generate_crate_notice "$release_dir/about-relay.toml" "$release_dir/about.hbs" "$output"

{
  unlicensed_crates_section
  native_libraries_section
} >>"$output"

require_text "$output" "Apache License" "MIT License"
echo "wrote $output ($(wc -c <"$output") bytes)"
