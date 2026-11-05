#!/usr/bin/env bash
# Decides what a release workflow run is, and refuses runs that must not publish.
# Writes version, tag, publish and prerelease to $GITHUB_OUTPUT.
#
#   publish   true only for a `push` of a tag. workflow_dispatch NEVER publishes, whatever
#             ref it runs on: a manual run is always a dry run.
#   version   the workspace version in Cargo.toml (the binary reports it). When publishing
#             it must equal the tag without its `v`, and deploy/compose/.env.example must
#             pin it with exactly one DESKHAVEN_TAG line, so the compose files at tag
#             vX.Y.Z pull the images of that very release.
#
# When publishing it also fails if the tagged commit is not on origin/main, or an image
# tagged with the version exists (a re-run must never retag images). That is answered by
# the GitHub Packages API: a 404 (no such package) is the only "free" answer, every other
# failure fails the run. (That a release for the tag exists is checked by the preflight
# job, which can see drafts; which tags may move is decided by the promote job.)
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

# package_has_tag PACKAGE TAG: 0 if the container package has the tag, 1 if the package
# has not been published yet or lacks the tag; anything else fails the run.
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
