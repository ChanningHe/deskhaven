#!/usr/bin/env bash
# release-publish.sh: existing release handling (draft / foreign / its own published one),
# tag checks before creating and before publishing, draft by API with an id, uploads by id,
# verification of assets and their GitHub digests, publish by id with make_latest. The gh
# stub runs the script's real --jq filters over JSON.
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
  set_releases
  printf 'b' >"$assets/deskhaven-v0.1.0-x86_64-unknown-linux-musl.tar.gz"
  printf 'c' >"$assets/THIRD-PARTY-NOTICES.txt"
  (cd "$assets" && sha256sum ./*.gz ./*.txt | sed 's| \./| |' >SHA256SUMS)
  echo "notes" >"$TMP/body.md"
}
# an existing release page entry: id 777, tag, draft?, with the assets and their real digests
existing_release() { # TAG DRAFT(true|false) [skip-asset-name] [bad-digest-name]
  local tag="$1" draft="$2" skip="${3:-}" bad="${4:-}" f name digest list="[]"
  for f in "$assets"/*; do
    name="$(basename "$f")"
    [ "$name" != "$skip" ] || continue
    digest="sha256:$(sha256sum "$f" | cut -d' ' -f1)"
    [ "$name" != "$bad" ] || digest="sha256:$(printf '0%.0s' $(seq 1 64))"
    list="$(jq -c --arg n "$name" --arg d "$digest" '. + [{name: $n, digest: $d}]' <<<"$list")"
  done
  mkdir -p "$STUB_DIR/releases"
  jq -n -c --arg t "$tag" --argjson d "$draft" --argjson a "$list" '[{id: 777, tag_name: $t, draft: $d, prerelease: false, assets: $a}]' >"$STUB_DIR/releases/page-1.json"
}
publish() { # TAG [ENV=VAL...]
  local tag="$1"
  shift
  env GITHUB_REF_NAME="$tag" VERSION="${tag#v}" PRERELEASE=false "$@" bash "$RELEASE_DIR/release-publish.sh" "$assets" "$TMP/body.md"
}
calls() { if [ -f "$STUB_DIR/gh.log" ]; then grep -c -- "$1" "$STUB_DIR/gh.log" || true; else echo 0; fi; }
asset_names() { jq -r '[.assets[].name] | sort | join(" ")' "$STUB_DIR/release.json"; }

echo "== a clean publish"
reset
run_capture p publish v0.1.0
assert_eq "publishes" "0" "$p_rc"
assert_eq "one draft was created" "1" "$(calls '--method POST repos/')"
assert_eq "all assets (and SHA256SUMS) were uploaded" "SHA256SUMS THIRD-PARTY-NOTICES.txt deskhaven-v0.1.0-x86_64-unknown-linux-musl.tar.gz" "$(asset_names)"
assert_eq "assets went to the release by id through the uploads API" "3" "$(calls 'https://uploads.github.com/repos/ChanningHe/deskhaven/releases/4242/assets?name=')"
assert_eq "published by id (no longer a draft)" "false" "$(jq -r .draft "$STUB_DIR/release.json")"
assert_contains "PATCH addresses the id" "releases/4242" "$(head -n 1 "$STUB_DIR/release.patch")"
assert_contains "the newest final release is Latest" "make_latest=true" "$(cat "$STUB_DIR/release.patch")"
assert_not_contains "no lookup of the release by tag" "release upload" "$(cat "$STUB_DIR/gh.log")"

echo "== refusals happen before anything is created"
reset
existing_release v0.1.0 true
run_capture p publish v0.1.0
assert_eq "a draft for the tag exists" "1" "$p_rc"
assert_contains "...says so" "already exists" "$p_out"
assert_eq "...and creates nothing" "0" "$(calls '--method POST repos/')"
reset
existing_release v0.1.0 false skip-none THIRD-PARTY-NOTICES.txt
run_capture p publish v0.1.0
assert_eq "a published release with other digests is foreign" "1" "$p_rc"
reset
existing_release v0.1.0 false deskhaven-v0.1.0-x86_64-unknown-linux-musl.tar.gz
run_capture p publish v0.1.0
assert_eq "a published release with other assets is foreign" "1" "$p_rc"
reset
run_capture p publish v0.1.0 STUB_API_ERROR=1
assert_eq "an API error while listing fails" "1" "$p_rc"
assert_eq "...and creates nothing" "0" "$(calls '--method POST repos/')"
reset
run_capture p publish v0.1.0 STUB_TAG_SHA=ffffffffffffffffffffffffffffffffffffffff
assert_eq "a tag moved to another commit" "1" "$p_rc"
assert_contains "...says so" "not the built" "$p_out"
assert_eq "...and creates nothing" "0" "$(calls '--method POST repos/')"
reset
echo tampered >"$assets/THIRD-PARTY-NOTICES.txt"
run_capture p publish v0.1.0
assert_eq "assets that do not match SHA256SUMS" "1" "$p_rc"
assert_eq "...create nothing" "0" "$(calls '--method POST repos/')"

echo "== the tag is checked again right before publishing"
reset
run_capture p publish v0.1.0 STUB_TAG_FLIP_AFTER=1 STUB_TAG_SHA_LATER=ffffffffffffffffffffffffffffffffffffffff
assert_eq "a tag that moved while the draft was prepared stops the publish" "1" "$p_rc"
assert_contains "...says so" "moved while the release was being prepared" "$p_out"
assert_eq "...the release stays a draft" "true" "$(jq -r .draft "$STUB_DIR/release.json")"

echo "== re-running after the PATCH succeeded but the job failed"
reset
existing_release v0.1.0 false
run_capture p publish v0.1.0
assert_eq "its own published release is accepted" "0" "$p_rc"
assert_contains "...and says nothing needed doing" "already published" "$p_out"
assert_eq "...without creating or patching anything" "0" "$(calls '--method P')"
reset
existing_release v0.1.0 false
run_capture p publish v0.1.0 STUB_TAG_SHA=ffffffffffffffffffffffffffffffffffffffff
assert_eq "...but not if the tag moved" "1" "$p_rc"

echo "== Latest badge and prerelease"
reset
set_releases v0.2.0:published
run_capture p publish v0.1.1
assert_contains "an older line's patch is not Latest" "make_latest=false" "$(cat "$STUB_DIR/release.patch")"
reset
run_capture p publish v0.2.0-rc.1 PRERELEASE=true
assert_contains "a prerelease is never Latest" "make_latest=false" "$(cat "$STUB_DIR/release.patch")"
assert_contains "...and is created as a prerelease" "prerelease=true" "$(cat "$STUB_DIR/gh.log")"

echo "== the draft is verified before publishing"
reset
run_capture p publish v0.1.0 STUB_UPLOAD_FAIL=1
assert_eq "an upload failure fails the run" "1" "$p_rc"
assert_eq "...leaving a draft" "true" "$(jq -r .draft "$STUB_DIR/release.json")"
reset
run_capture p publish v0.1.0 STUB_UPLOAD_SKIP=THIRD-PARTY-NOTICES.txt
assert_eq "an asset that did not arrive stops the publish" "1" "$p_rc"
assert_eq "...the release stays a draft" "true" "$(jq -r .draft "$STUB_DIR/release.json")"
reset
run_capture p publish v0.1.0 STUB_UPLOAD_CORRUPT=THIRD-PARTY-NOTICES.txt
assert_eq "an asset whose GitHub digest differs from SHA256SUMS stops the publish" "1" "$p_rc"
assert_eq "...the release stays a draft" "true" "$(jq -r .draft "$STUB_DIR/release.json")"
reset
run_capture p publish v0.1.0 STUB_NO_DIGEST=1
assert_eq "assets without a digest cannot be verified: stop" "1" "$p_rc"
assert_eq "...the release stays a draft" "true" "$(jq -r .draft "$STUB_DIR/release.json")"
reset
run_capture p publish v0.1.0 STUB_PATCH_IGNORED=1
assert_eq "a publish that did not take effect fails" "1" "$p_rc"
assert_contains "...says it is still a draft" "still a draft" "$p_out"
finish
