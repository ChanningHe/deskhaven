#!/usr/bin/env bash
# exact-tags.sh (the manifest job's guard): an existing X.Y.Z that is exactly this run is
# accepted, a different one, a moved git tag, a wrong revision or a bad digest is refused
# BEFORE any index is created.
set -euo pipefail
# shellcheck source=lib.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
use_stubs
export STUB_REG="$TMP/reg"
export GITHUB_REPOSITORY_OWNER=ChanningHe CI_REPO_URL=unused VERSION=0.1.0 GITHUB_REF_NAME=v0.1.0
SHA=1234567890123456789012345678901234567890
export GITHUB_SHA="$SHA" STUB_TAG_SHA="$SHA"

digests="$TMP/digests"
leaf() { fixture_digest "$1-$2"; } # image arch
reg_init() {
  rm -rf "$STUB_REG" "$digests"
  mkdir -p "$STUB_REG/tags/deskhaven" "$STUB_REG/tags/deskhaven-relay" "$STUB_REG/idx" "$STUB_REG/rev" "$digests"
  local key image arch
  for key in deskhaven:deskhaven relay:deskhaven-relay; do
    image="${key#*:}"
    for arch in amd64 arm64; do
      echo "$SHA" >"$STUB_REG/rev/$(leaf "$image" "$arch")"
      echo "$(leaf "$image" "$arch")" >"$digests/${key%%:*}-${arch}.txt"
    done
  done
}
# existing_tag IMAGE [swap]: the exact tag already exists; with "swap" it lists other children
existing_tag() {
  local image="$1" idx a b
  idx="$(fixture_digest "existing-$1")"
  a="$(leaf "$image" amd64)"
  b="$(leaf "$image" arm64)"
  if [ "${2:-}" = swap ]; then b="$(fixture_digest "someone-elses-$image")"; echo "$SHA" >"$STUB_REG/rev/$b"; fi
  printf '%s\n%s\n' "$b" "$a" >"$STUB_REG/idx/$idx" # reversed order on purpose: compared sorted
  echo "$idx" >"$STUB_REG/tags/$image/0.1.0"
}
creates() { if [ -f "$STUB_REG/docker.log" ]; then grep -c "imagetools create" "$STUB_REG/docker.log" || true; else echo 0; fi; }
run() { bash "$RELEASE_DIR/exact-tags.sh" "$digests" "$TMP/images.txt"; }

echo "== fresh run"
reg_init
expect_pass "creates the exact tag for both images" run
assert_eq "two indexes created" "2" "$(creates)"
assert_eq "images.txt lists both" "2" "$(wc -l <"$TMP/images.txt" | tr -d ' ')"
for image in deskhaven deskhaven-relay; do
  [ -f "$STUB_REG/tags/$image/0.1.0" ] && ok "$image:0.1.0 exists" || bad "$image:0.1.0 missing"
done

echo "== an annotated git tag (peeled) is fine"
reg_init
run_capture r env STUB_TAG_ANNOTATED=1 bash "$RELEASE_DIR/exact-tags.sh" "$digests" "$TMP/images.txt"
assert_eq "annotated tag" "0" "$r_rc"

echo "== existing X.Y.Z"
reg_init; existing_tag deskhaven; existing_tag deskhaven-relay
expect_pass "existing with exactly this run's digests is a re-run, accepted" run
assert_eq "...and nothing is created" "0" "$(creates)"
reg_init; existing_tag deskhaven
expect_pass "one image done, the other missing: the re-run completes it" run
assert_eq "...creating only the missing one" "1" "$(creates)"
reg_init; existing_tag deskhaven; existing_tag deskhaven-relay swap
expect_fail "existing with different digests is refused" "already exists with different content" run
assert_eq "...before the other image gets an index" "0" "$(creates)"
reg_init; existing_tag deskhaven swap
expect_fail "the first image differing refuses too" "already exists with different content" run
assert_eq "...and creates nothing" "0" "$(creates)"

echo "== only buildx's exact not-found text means 'absent'"
reg_init
existing_tag deskhaven
expect_fail "a transient error containing 404 and 'not found' fails closed" "cannot tell whether" env STUB_FLAKY_ERROR=1 bash "$RELEASE_DIR/exact-tags.sh" "$digests" "$TMP/images.txt"
assert_eq "...and creates nothing, so the existing tag is not overwritten" "0" "$(creates)"
assert_eq "...the published tag is untouched" "$(fixture_digest "existing-deskhaven")" "$(cat "$STUB_REG/tags/deskhaven/0.1.0")"
reg_init
expect_fail "an unreachable registry fails closed too" "no such host" env STUB_REGISTRY_ERROR=1 bash "$RELEASE_DIR/exact-tags.sh" "$digests" "$TMP/images.txt"

echo "== the create is verified"
reg_init
expect_fail "a create that lands on other children is not recorded" "does not list this run's images after the create" env STUB_CREATE_WRONG=1 bash "$RELEASE_DIR/exact-tags.sh" "$digests" "$TMP/images.txt"

echo "== the git tag"
reg_init
expect_fail "tag moved to another commit" "no longer points to the built commit" env STUB_TAG_SHA=ffffffffffffffffffffffffffffffffffffffff bash "$RELEASE_DIR/exact-tags.sh" "$digests" "$TMP/images.txt"
assert_eq "...nothing created" "0" "$(creates)"
expect_fail "tag deleted" "no longer points to the built commit" env STUB_TAG_SHA= bash "$RELEASE_DIR/exact-tags.sh" "$digests" "$TMP/images.txt"

echo "== digests and revisions"
reg_init
echo "$(fixture_digest wrong)" >"$STUB_REG/rev/$(leaf deskhaven-relay arm64)"
expect_fail "a per-arch image of another commit" "does not carry revision" run
assert_eq "...nothing created" "0" "$(creates)"
reg_init
echo "sha256:short" >"$digests/relay-arm64.txt"
expect_fail "a malformed digest" "invalid digest" run
reg_init
rm "$digests/deskhaven-amd64.txt"
expect_fail "a missing digest file" "missing digest file" run
finish
