#!/usr/bin/env bash
# promote.sh decide and apply: the decision table and the registry scenarios (converging,
# self-healing, refusing on a revision mismatch, rolling back on a failure midway), against
# stubbed gh, docker and git ls-remote.
set -euo pipefail
# shellcheck source=lib.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
use_stubs
export STUB_DIR="$TMP/stub" STUB_REG="$TMP/reg" STUB_TAG_FILE="$TMP/stub/tags.txt"
mkdir -p "$STUB_DIR"
export GITHUB_REPOSITORY=ChanningHe/deskhaven GITHUB_REPOSITORY_OWNER=ChanningHe CI_REPO_URL=unused

promote() { env VERSION="$1" bash "$RELEASE_DIR/promote.sh" "$2"; }
# published TAG...: those are the published final releases. The fixture ALSO holds a draft
# v99.0.0 and a prerelease v98.0.0-rc.1, which must never count (pages of 2 releases each,
# so the filter is applied per page).
published() {
  local specs=() t
  for t in "$@"; do specs+=("$t:published"); done
  set_releases "${specs[@]}" v99.0.0:draft v98.0.0-rc.1:prerelease
}
decide() { promote "$1" decide | tr '\n' ' '; }

echo "== decide"
published
assert_eq "first release (nothing published yet)" "latest=true minor_latest=true move_0.1=v0.1.0 move_latest=v0.1.0 " "$(decide 0.1.0)"
published v0.1.0 v0.2.0
assert_eq "old-line patch" "latest=false minor_latest=true move_0.1=v0.1.1 move_0.2=v0.2.0 move_latest=v0.2.0 " "$(decide 0.1.1)"
published v0.2.0
assert_eq "prerelease" "latest=false minor_latest=false move_0.2=v0.2.0 move_latest=v0.2.0 " "$(decide 0.3.0-rc.1)"
published v0.1.0 v0.2.0 v0.2.1 v0.3.0
assert_eq "stale re-run of v0.2.0 after v0.2.1 and v0.3.0" "latest=false minor_latest=false move_0.2=v0.2.1 move_0.3=v0.3.0 move_latest=v0.3.0 " "$(decide 0.2.0)"
published v0.1.0 v0.2.0
printf 'v0.1.0 aaa\nv0.2.0 bbb\nv10.0.0 ccc\n' >"$STUB_TAG_FILE"
assert_eq "a stray unreleased git tag v10.0.0 does not freeze latest" "latest=true minor_latest=true move_0.2=v0.2.0 move_latest=v0.2.0 " "$(decide 0.2.0)"
run_capture d env STUB_API_ERROR=1 VERSION=0.2.0 bash "$RELEASE_DIR/promote.sh" decide
assert_eq "an unreadable release list fails" "1" "$d_rc"

# ---- registry fixtures
sha_of() { printf '%040d' "$(printf '%s' "$1" | cksum | cut -d' ' -f1)"; }
reg_init() { rm -rf "$STUB_REG"; mkdir -p "$STUB_REG/tags/deskhaven" "$STUB_REG/tags/deskhaven-relay" "$STUB_REG/idx" "$STUB_REG/rev"; : >"$STUB_TAG_FILE"; }
# add_version VERSION: both images get the exact tag; the git tag points to sha_of(VERSION)
add_version() {
  local version="$1" image a b idx
  echo "v${version} $(sha_of "$version")" >>"$STUB_TAG_FILE"
  for image in deskhaven deskhaven-relay; do
    a="$(fixture_digest "${image}-${version}-amd64")"
    b="$(fixture_digest "${image}-${version}-arm64")"
    idx="$(fixture_digest "${image}-${version}-index")"
    sha_of "$version" >"$STUB_REG/rev/$a"
    sha_of "$version" >"$STUB_REG/rev/$b"
    printf '%s\n%s\n' "$a" "$b" >"$STUB_REG/idx/$idx"
    echo "$idx" >"$STUB_REG/tags/$image/$version"
  done
}
idx_of() { fixture_digest "$1-$2-index"; } # image version
set_tag() { echo "$(idx_of "$1" "$3")" >"$STUB_REG/tags/$1/$2"; } # image tag version
tag_is() { # image tag version -> 0 if the tag points to that version's exact digest
  [ "$(cat "$STUB_REG/tags/$1/$2" 2>/dev/null || true)" = "$(idx_of "$1" "$3")" ]
}
state() { # one line: where the given tags of both images point
  local image t out=""
  for image in deskhaven deskhaven-relay; do
    for t in "$@"; do
      out+="$image/$t=$(cut -c1-13 "$STUB_REG/tags/$image/$t" 2>/dev/null || echo none) "
    done
  done
  echo "$out"
}
apply_ok() { run_capture a promote "$1" apply; }

echo "== apply: first release"
reg_init; add_version 0.1.0; published v0.1.0
apply_ok 0.1.0
assert_eq "first release promotes" "0" "$a_rc"
for image in deskhaven deskhaven-relay; do
  tag_is "$image" 0.1 0.1.0 && ok "$image 0.1 -> 0.1.0" || bad "$image 0.1 not at 0.1.0"
  tag_is "$image" latest 0.1.0 && ok "$image latest -> 0.1.0" || bad "$image latest not at 0.1.0"
done

echo "== apply: old-line patch leaves latest alone"
reg_init; add_version 0.1.0; add_version 0.2.0; add_version 0.1.1
for image in deskhaven deskhaven-relay; do set_tag "$image" latest 0.2.0; set_tag "$image" 0.1 0.1.0; set_tag "$image" 0.2 0.2.0; done
published v0.1.0 v0.2.0 v0.1.1
apply_ok 0.1.1
assert_eq "patch of the old line promotes" "0" "$a_rc"
for image in deskhaven deskhaven-relay; do
  tag_is "$image" 0.1 0.1.1 && ok "$image 0.1 -> 0.1.1" || bad "$image 0.1 not at 0.1.1"
  tag_is "$image" latest 0.2.0 && ok "$image latest stays 0.2.0" || bad "$image latest moved"
done

echo "== apply: prerelease"
reg_init; add_version 0.3.0-rc.1; published v0.2.0
apply_ok 0.3.0-rc.1
assert_eq "prerelease exits cleanly" "0" "$a_rc"
assert_contains "prerelease says nothing to promote" "nothing to promote" "$a_out"
[ ! -e "$STUB_REG/tags/deskhaven/latest" ] && ok "prerelease moves no tag" || bad "prerelease created latest"

echo "== apply: stale re-run does not move tags backwards"
reg_init; add_version 0.1.0; add_version 0.2.0; add_version 0.2.1; add_version 0.3.0
for image in deskhaven deskhaven-relay; do set_tag "$image" latest 0.3.0; set_tag "$image" 0.2 0.2.1; set_tag "$image" 0.3 0.3.0; done
published v0.1.0 v0.2.0 v0.2.1 v0.3.0
apply_ok 0.2.0
assert_eq "stale re-run succeeds" "0" "$a_rc"
for image in deskhaven deskhaven-relay; do
  tag_is "$image" 0.2 0.2.1 && ok "$image 0.2 stays 0.2.1" || bad "$image 0.2 moved backwards"
  tag_is "$image" latest 0.3.0 && ok "$image latest stays 0.3.0" || bad "$image latest moved backwards"
done

echo "== apply: self-heal of a missed earlier promote"
reg_init; add_version 0.1.0; add_version 0.2.0
for image in deskhaven deskhaven-relay; do set_tag "$image" latest 0.1.0; set_tag "$image" 0.1 0.1.0; done
published v0.1.0 v0.2.0
apply_ok 0.1.0 # an old run's promote, run late: v0.2.0's promote was missed
assert_eq "self-heal run succeeds" "0" "$a_rc"
for image in deskhaven deskhaven-relay; do
  tag_is "$image" latest 0.2.0 && ok "$image latest healed to 0.2.0" || bad "$image latest not healed"
  tag_is "$image" 0.2 0.2.0 && ok "$image 0.2 created at 0.2.0" || bad "$image 0.2 not healed"
done

echo "== apply: revision mismatch refuses, nothing moves"
reg_init; add_version 0.1.0; add_version 0.2.0
for image in deskhaven deskhaven-relay; do set_tag "$image" latest 0.1.0; set_tag "$image" 0.1 0.1.0; done
published v0.1.0 v0.2.0
sed -i 's/^v0.2.0 .*/v0.2.0 ffffffffffffffffffffffffffffffffffffffff/' "$STUB_TAG_FILE" # the git tag was moved
before="$(state latest 0.1)"
apply_ok 0.2.0
if [ "$a_rc" != 0 ] && grep -q "does not carry revision" <<<"$a_out"; then ok "a retagged git tag is refused"; else bad "mismatch not refused: rc=$a_rc $a_out"; fi
assert_eq "no tag moved" "$before" "$(state latest 0.1)"
reg_init; add_version 0.1.0; published v0.1.0
echo "$(fixture_digest other)" >"$STUB_REG/rev/$(head -n 1 "$STUB_REG/idx/$(idx_of deskhaven 0.1.0)")" # the image carries another revision
apply_ok 0.1.0
if [ "$a_rc" != 0 ] && grep -q "does not carry revision" <<<"$a_out"; then ok "an image of another commit is refused"; else bad "image mismatch not refused: rc=$a_rc"; fi

echo "== apply: refusals"
reg_init; add_version 0.1.0; published v0.1.0
apply_ok 0.3.0
if [ "$a_rc" != 0 ] && grep -q "not a published release" <<<"$a_out"; then ok "an unpublished version is refused"; else bad "unpublished not refused: $a_out"; fi
reg_init; published v0.1.0; echo "v0.1.0 $(sha_of 0.1.0)" >"$STUB_TAG_FILE"
apply_ok 0.1.0
assert_eq "a published release without images fails" "1" "$a_rc"

echo "== apply: a failure midway rolls back, both images stay consistent"
for fail_n in 3 4; do
  reg_init; add_version 0.1.0; add_version 0.2.0
  for image in deskhaven deskhaven-relay; do set_tag "$image" latest 0.1.0; set_tag "$image" 0.2 0.1.0; done
  published v0.1.0 v0.2.0
  before="$(state latest 0.2)"
  run_capture a env STUB_FAIL_CREATE_N="$fail_n" VERSION=0.2.0 bash "$RELEASE_DIR/promote.sh" apply
  assert_eq "create #$fail_n fails: the run fails" "1" "$a_rc"
  assert_contains "create #$fail_n fails: restoring announced" "restoring every tag that was attempted" "$a_out"
  assert_eq "create #$fail_n fails: both images are back where they were" "$before" "$(state latest 0.2)"
done
reg_init; add_version 0.1.0; add_version 0.2.0
for image in deskhaven deskhaven-relay; do set_tag "$image" latest 0.1.0; set_tag "$image" 0.2 0.1.0; done
published v0.1.0 v0.2.0
apply_ok 0.2.0
assert_eq "then a clean run completes the move" "0" "$a_rc"
for image in deskhaven deskhaven-relay; do
  tag_is "$image" latest 0.2.0 && ok "$image latest -> 0.2.0" || bad "$image latest wrong"
  tag_is "$image" 0.2 0.2.0 && ok "$image 0.2 -> 0.2.0" || bad "$image 0.2 wrong"
done
before_log="$(grep -c 'imagetools create' "$STUB_REG/docker.log")"
apply_ok 0.2.0
assert_eq "an idempotent re-run succeeds" "0" "$a_rc"
assert_eq "...and creates nothing" "$before_log" "$(grep -c 'imagetools create' "$STUB_REG/docker.log")"

echo "== a create the registry applied but reported as failed is rolled back too"
export PROMOTE_RETRY_SLEEP=0
reg_init; add_version 0.1.0; add_version 0.2.0
for image in deskhaven deskhaven-relay; do set_tag "$image" latest 0.1.0; set_tag "$image" 0.2 0.1.0; done
published v0.1.0 v0.2.0
before="$(state latest 0.2)"
run_capture a env STUB_APPLY_THEN_FAIL_N=4 VERSION=0.2.0 bash "$RELEASE_DIR/promote.sh" apply
assert_eq "the run fails" "1" "$a_rc"
assert_eq "the in-flight 4th move (relay latest, applied) is undone: both images are where they were" "$before" "$(state latest 0.2)"
assert_contains "the rollback reports a clean state" "rolled back: every touched tag is where it was" "$a_out"
assert_not_contains "no INCONSISTENT list" "INCONSISTENT" "$a_out"

echo "== a rollback that cannot restore says so, after retrying"
reg_init; add_version 0.1.0; add_version 0.2.0
for image in deskhaven deskhaven-relay; do set_tag "$image" latest 0.1.0; set_tag "$image" 0.2 0.1.0; done
published v0.1.0 v0.2.0
run_capture a env STUB_FAIL_CREATE_FROM=3 PROMOTE_RETRY_ATTEMPTS=3 VERSION=0.2.0 bash "$RELEASE_DIR/promote.sh" apply
assert_eq "the run fails" "1" "$a_rc"
assert_contains "each restore was retried" "(attempt 3/3)" "$a_out"
assert_contains "an explicit INCONSISTENT list names the tag" "INCONSISTENT: ghcr.io/channinghe/deskhaven:0.2" "$a_out"
assert_eq "and the exit status is non-zero" "1" "$a_rc"

echo "== tags are moved from the verified digest, every platform is checked"
reg_init; add_version 0.1.0; published v0.1.0
apply_ok 0.1.0
assert_contains "create uses name@digest" "--tag ghcr.io/channinghe/deskhaven:latest ghcr.io/channinghe/deskhaven@sha256:" "$(cat "$STUB_REG/docker.log")"
assert_not_contains "...not the mutable name:version" "ghcr.io/channinghe/deskhaven:latest ghcr.io/channinghe/deskhaven:0.1.0" "$(cat "$STUB_REG/docker.log")"
reg_init; add_version 0.1.0; published v0.1.0
echo "$(fixture_digest other)" >"$STUB_REG/rev/$(sed -n 2p "$STUB_REG/idx/$(idx_of deskhaven 0.1.0)")" # only the SECOND platform carries another revision
apply_ok 0.1.0
if [ "$a_rc" != 0 ] && grep -q "on every platform" <<<"$a_out"; then ok "one platform of another commit is refused"; else bad "second platform not checked: rc=$a_rc $a_out"; fi
finish
