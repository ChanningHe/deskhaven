#!/usr/bin/env bash
# Decides what a release workflow run is, and refuses runs that must not publish.
# Writes version, tag, publish, prerelease, latest and minor_latest to $GITHUB_OUTPUT.
#
#   publish   true only for a `push` of a tag. workflow_dispatch NEVER publishes, whatever
#             ref it runs on: a manual run is always a dry run.
#   version   the workspace version in Cargo.toml (the binary reports it). When publishing
#             it must equal the tag without its `v`, and deploy/compose/.env.example must
#             pin it (DESKHAVEN_TAG), so the compose files at tag vX.Y.Z pull the images
#             of that very release.
#   latest / minor_latest   whether the image tags `latest` and `X.Y` may move to this
#             release (tags.sh plan).
#
# When publishing it also fails if the tagged commit is not on origin/main, a release for
# the tag exists, or an image tagged with the version exists (a re-run must never retag
# images or replace a release).
#
# Inputs (environment): GITHUB_EVENT_NAME, GITHUB_REF_TYPE, GITHUB_REF_NAME, GITHUB_SHA,
# GITHUB_REPOSITORY_OWNER, GITHUB_OUTPUT (default: stdout), GH_TOKEN (gh). The working
# directory is the repository with all tags and origin/main fetched.
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
  grep -qxF "DESKHAVEN_TAG=$version" deploy/compose/.env.example ||
    fail "deploy/compose/.env.example must contain DESKHAVEN_TAG=$version (the compose files at this tag must pin this release)"
else
  version="$cargo_version"
fi

prerelease=false
if [[ "$version" == *-* ]]; then prerelease=true; fi
tag="v$version"

plan="$("$root/.github/release/tags.sh" plan "$tag")"
latest="$(sed -n 's/^latest=//p' <<<"$plan")"
minor_latest="$(sed -n 's/^minor_latest=//p' <<<"$plan")"

if [ "$publish" = true ]; then
  git merge-base --is-ancestor "$GITHUB_SHA" origin/main ||
    fail "$GITHUB_SHA is not on main: release from a commit that main contains"

  if gh release view "$GITHUB_REF_NAME" >/dev/null 2>&1; then
    fail "a release for $GITHUB_REF_NAME already exists; a release is never replaced (delete it, or tag a new version)"
  fi

  owner="${GITHUB_REPOSITORY_OWNER,,}"
  for image in deskhaven deskhaven-relay; do
    ref="ghcr.io/${owner}/${image}:${version}"
    if inspected="$(docker buildx imagetools inspect "$ref" 2>&1)"; then
      fail "$ref already exists; an image tag is never moved (tag a new version)"
    fi
    # A missing tag or package is the expected answer; anything else (network, auth
    # that a successful login should have prevented) must not be read as "free".
    grep -qiE 'not found|manifest unknown|name unknown|denied|unauthorized|404' <<<"$inspected" ||
      fail "cannot tell whether $ref exists: $inspected"
  done
fi

{
  echo "version=$version"
  echo "tag=$tag"
  echo "publish=$publish"
  echo "prerelease=$prerelease"
  echo "latest=$latest"
  echo "minor_latest=$minor_latest"
} >>"$output"
echo "version $version, publish $publish, prerelease $prerelease, latest $latest, minor_latest $minor_latest"
