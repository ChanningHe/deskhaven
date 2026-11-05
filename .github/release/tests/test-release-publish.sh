#!/usr/bin/env bash
# release-publish.sh: preflight repeated, tag check, draft by API with an id, upload,
# verification by id, publish by id with make_latest.
set -euo pipefail
# shellcheck source=lib.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
use_stubs
export STUB_DIR="$TMP/stub"
SHA=1234567890123456789012345678901234567890
export GITHUB_REPOSITORY=ChanningHe/deskhaven GITHUB_REPOSITORY_OWNER=ChanningHe CI_REPO_URL=unused
export GITHUB_SHA="$SHA" STUB_TAG_SHA="$SHA"

assets="$TMP/assets"
reset() {
  rm -rf "$STUB_DIR" "$assets"
  mkdir -p "$STUB_DIR" "$assets"
  : >"$STUB_DIR/all_tags.txt"
  : >"$STUB_DIR/published_final.txt"
  printf 'a' >"$assets/SHA256SUMS"
  printf 'b' >"$assets/deskhaven-v0.1.0-x86_64-unknown-linux-musl.tar.gz"
  printf 'c' >"$assets/THIRD-PARTY-NOTICES.txt"
  echo "notes" >"$TMP/body.md"
}
publish() { # TAG [ENV=VAL...]
  local tag="$1"
  shift
  env GITHUB_REF_NAME="$tag" VERSION="${tag#v}" PRERELEASE=false "$@" bash "$RELEASE_DIR/release-publish.sh" "$assets" "$TMP/body.md"
}
posts() { if [ -f "$STUB_DIR/gh.log" ]; then grep -c -- '--method POST' "$STUB_DIR/gh.log" || true; else echo 0; fi; }

echo "== a clean publish"
reset
run_capture p publish v0.1.0
assert_eq "publishes" "0" "$p_rc"
assert_eq "one draft was created" "1" "$(posts)"
assert_eq "all assets were uploaded" "3" "$(wc -l <"$STUB_DIR/release.assets" | tr -d ' ')"
assert_eq "published by id (no longer a draft)" "false" "$(cat "$STUB_DIR/release.draft")"
assert_contains "PATCH addresses the id" "/releases/4242" "$(cat "$STUB_DIR/release.patch")"
assert_contains "the newest final release is Latest" "make_latest=true" "$(cat "$STUB_DIR/release.patch")"
assert_not_contains "no publish via the tag" "release edit" "$(cat "$STUB_DIR/gh.log")"
assert_contains "the draft carries the body from the file" "body=@$TMP/body.md" "$(cat "$STUB_DIR/gh.log")"

echo "== refusals happen before anything is created"
reset
echo v0.1.0 >"$STUB_DIR/all_tags.txt"
run_capture p publish v0.1.0
assert_eq "a draft or release for the tag exists" "1" "$p_rc"
assert_contains "...says so" "already exists" "$p_out"
assert_eq "...and creates nothing" "0" "$(posts)"
reset
run_capture p publish v0.1.0 STUB_API_ERROR=1
assert_eq "an API error while listing fails" "1" "$p_rc"
assert_eq "...and creates nothing" "0" "$(posts)"
reset
run_capture p publish v0.1.0 STUB_TAG_SHA=ffffffffffffffffffffffffffffffffffffffff
assert_eq "a tag moved to another commit" "1" "$p_rc"
assert_contains "...says so" "not the built" "$p_out"
assert_eq "...and creates nothing" "0" "$(posts)"

echo "== Latest badge and prerelease"
reset
echo v0.2.0 >"$STUB_DIR/published_final.txt"
run_capture p publish v0.1.1 STUB_TAG_SHA="$SHA"
assert_contains "an older line's patch is not Latest" "make_latest=false" "$(cat "$STUB_DIR/release.patch")"
reset
run_capture p publish v0.2.0-rc.1 PRERELEASE=true
assert_contains "a prerelease is never Latest" "make_latest=false" "$(cat "$STUB_DIR/release.patch")"
assert_contains "...and is created as a prerelease" "prerelease=true" "$(cat "$STUB_DIR/gh.log")"

echo "== the draft is verified before publishing"
reset
run_capture p publish v0.1.0 STUB_UPLOAD_FAIL=1
assert_eq "an upload failure fails the run" "1" "$p_rc"
assert_eq "...leaving a draft" "true" "$(cat "$STUB_DIR/release.draft")"
reset
run_capture p publish v0.1.0 STUB_UPLOAD_SKIP=THIRD-PARTY-NOTICES.txt
assert_eq "an asset that did not arrive stops the publish" "1" "$p_rc"
assert_eq "...the release stays a draft" "true" "$(cat "$STUB_DIR/release.draft")"
finish
