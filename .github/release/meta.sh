#!/usr/bin/env bash
# Decides what a release workflow run is, and refuses runs that must not publish.
# Writes version, tag, publish and prerelease to $GITHUB_OUTPUT.
# workflow_dispatch never publishes. .env.example must pin the version so the compose
# files at the tag pull that very release; an existing image tag fails (never retag).
#
# Inputs (environment): GITHUB_EVENT_NAME, GITHUB_REF_TYPE, GITHUB_REF_NAME, GITHUB_SHA,
# GITHUB_REPOSITORY, GITHUB_REPOSITORY_OWNER, GITHUB_OUTPUT (default: stdout), GH_TOKEN
# (gh, packages: read). The working directory is the repository with origin/main fetched.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
output="${GITHUB_OUTPUT:-/dev/stdout}"

fail() {
  echo "::error::$*"
  exit 1
}

cargo_version="$(sed -nE '/^\[workspace\.package\]/,/^\[/{s/^version = "([^"]+)".*/\1/p}' Cargo.toml)"
[ -n "$cargo_version" ] || fail "workspace version not found in Cargo.toml"

publish=false
if [ "${GITHUB_EVENT_NAME:-}" = push ] && [ "${GITHUB_REF_TYPE:-}" = tag ]; then
  publish=true
fi

if [ "$publish" = true ]; then
  semver='^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z]+(\.[0-9A-Za-z]+)*)?$'
  [[ "$GITHUB_REF_NAME" =~ $semver ]] || fail "tag '$GITHUB_REF_NAME' is not vX.Y.Z[-prerelease]"
  version="${GITHUB_REF_NAME#v}"
  [ "$version" = "$cargo_version" ] ||
    fail "tag $GITHUB_REF_NAME does not match the workspace version $cargo_version in Cargo.toml"
  [ "$(grep -c '^DESKHAVEN_TAG=' deploy/compose/.env.example)" = 1 ] ||
    fail "deploy/compose/.env.example must have exactly one DESKHAVEN_TAG= line"
  grep -qxF "DESKHAVEN_TAG=$version" deploy/compose/.env.example ||
    fail "deploy/compose/.env.example must contain DESKHAVEN_TAG=$version (the compose files at this tag must pin this release)"
else
  version="$cargo_version"
fi

prerelease=false
if [[ "$version" == *-* ]]; then prerelease=true; fi

package_has_tag() {
  local package="$1" wanted="$2" kind versions err
  err="$(mktemp)"
  kind="$(gh api "repos/${GITHUB_REPOSITORY}" --jq 'if .owner.type == "Organization" then "orgs" else "users" end')" ||
    fail "cannot read repos/${GITHUB_REPOSITORY}"
  if ! versions="$(gh api --paginate "${kind}/${GITHUB_REPOSITORY_OWNER}/packages/container/${package}/versions?per_page=100" \
    --jq '.[].metadata.container.tags[]' 2>"$err")"; then
    if grep -q 'HTTP 404' "$err"; then
      rm -f "$err"
      return 1
    fi
    fail "cannot tell whether ${package}:${wanted} exists: $(cat "$err")"
  fi
  rm -f "$err"
  grep -qxF "$wanted" <<<"$versions"
}

if [ "$publish" = true ]; then
  git merge-base --is-ancestor "$GITHUB_SHA" origin/main ||
    fail "$GITHUB_SHA is not on main: release from a commit that main contains"
  for package in deskhaven deskhaven-relay; do
    if package_has_tag "$package" "$version"; then
      fail "ghcr.io/${GITHUB_REPOSITORY_OWNER,,}/${package}:${version} already exists; an image tag is never moved (tag a new version)"
    fi
  done
fi

{
  echo "version=$version"
  echo "tag=v$version"
  echo "publish=$publish"
  echo "prerelease=$prerelease"
} >>"$output"
echo "version $version, publish $publish, prerelease $prerelease"
