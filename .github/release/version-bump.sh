#!/usr/bin/env bash
# Sets the release version everywhere it is pinned and commits the bump on main. Local
# only: push main yourself, wait for its ci run, then tag it with version-tag.sh.
# Docs links move for final versions only: they never point users at a pre-release.
#
# Usage: version-bump.sh X.Y.Z[-prerelease]   (run inside the repository, on main)
set -euo pipefail

fail() {
  echo "version-bump: $*" >&2
  exit 1
}

[ "$#" = 1 ] || fail "usage: version-bump.sh X.Y.Z[-prerelease]"
version="$1"
semver='^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z]+(\.[0-9A-Za-z]+)*)?$' # meta.sh's, without the v
[[ "$version" =~ $semver ]] || fail "'$version' is not X.Y.Z[-prerelease] (no leading v)"

root="$(git rev-parse --show-toplevel)" || fail "not inside a git repository"
cd "$root"
env_file=deploy/compose/.env.example

[ "$(git symbolic-ref --short -q HEAD || true)" = main ] || fail "not on main"
[ -z "$(git status --porcelain)" ] || fail "the working tree is not clean"
if git rev-parse -q --verify "refs/tags/v$version" >/dev/null; then
  fail "tag v$version exists: a version is never reused"
fi
current="$(sed -nE '/^\[workspace\.package\]/,/^\[/{s/^version = "([^"]+)".*/\1/p}' Cargo.toml)"
[ -n "$current" ] || fail "workspace version not found in Cargo.toml"
[ "$current" != "$version" ] || fail "the workspace version is already $version"
[ "$(grep -c '^DESKHAVEN_TAG=' "$env_file")" = 1 ] || fail "$env_file must have exactly one DESKHAVEN_TAG= line"

# The tree was clean, so a failure restores it and the bump can simply be re-run.
trap 'git checkout -q -- .; echo "version-bump: failed; the working tree was restored" >&2' ERR

sed -i -E '/^\[workspace\.package\]/,/^\[/{s/^version = "[^"]+"/version = "'"$version"'"/}' Cargo.toml
sed -i -E "s/^DESKHAVEN_TAG=.*/DESKHAVEN_TAG=$version/" "$env_file"
if [[ "$version" != *-* ]]; then
  any='v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z]+(\.[0-9A-Za-z]+)*)?'
  mapfile -t docs < <(grep -rlF 'ChanningHe/deskhaven/' docs)
  for doc in "${docs[@]}"; do
    sed -i -E -e "s#(ChanningHe/deskhaven/([A-Za-z]+/)*)$any#\\1v$version#g" \
      -e "s#\\[$any release\\]\\((https://github\\.com/ChanningHe/deskhaven/)#[v$version release](\\3#g" "$doc"
  done
fi
# Not --offline: a machine that never fetched the git dependencies needs the network.
cargo update --workspace
cargo xtask gen-openapi

git commit -q -a -m "chore: release v$version"
trap - ERR
echo "committed the bump to $version ($(git rev-parse --short HEAD)). Next: push main, wait for its ci run to pass, then tag it (just tag)."
