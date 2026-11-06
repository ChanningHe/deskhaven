#!/usr/bin/env bash
# version-tag.sh: every refusal leaves no tag anywhere, and a tag is pushed only for a clean
# main that GitHub has, pins its version and passed ci in a push run on main. A scratch
# repository with a local bare origin and a stubbed gh.
set -euo pipefail
# shellcheck source=lib.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
use_stubs
export STUB_DIR="$TMP/stub"
mkdir -p "$STUB_DIR"
export DESKHAVEN_REPO=ChanningHe/deskhaven

origin="$TMP/origin.git"
git init -q --bare "$origin"
repo="$TMP/repo"
mkdir -p "$repo/deploy/compose"
cd "$repo"
git init -q -b main .
git config user.email t@example.com
git config user.name t
git remote add origin "$origin"
printf '[workspace.package]\nversion = "0.2.0"\n\n[other]\nversion = "9.9.9"\n' >Cargo.toml
printf '# header\nDESKHAVEN_TAG=0.2.0\n' >deploy/compose/.env.example
git add -A
git commit -q -m "chore: release v0.2.0"
git push -q origin main
head="$(git rev-parse HEAD)"
export STUB_MAIN_SHA="$head"

tag_it() { bash "$RELEASE_DIR/version-tag.sh"; }
# run SHA EVENT BRANCH CONCLUSION: one ci.yml run object
run() { jq -n -c --arg s "$1" --arg e "$2" --arg b "$3" --arg c "$4" \
  '{id: 7, head_sha: $s, event: $e, head_branch: $b, conclusion: (if $c == "null" then null else $c end)}'; }
set_runs() { jq -s '{workflow_runs: .}' >"$STUB_DIR/runs.json"; }
set_refs() { printf '%s\n' "$@" | jq -R '{ref: .}' | jq -s . >"$STUB_DIR/refs.json"; }
green_run() { run "$head" push main success | set_runs; }
no_tag_anywhere() { # LABEL
  assert_eq "$1: no local tag" "" "$(git tag -l v0.2.0)"
  assert_eq "$1: no tag on origin" "" "$(git --git-dir="$origin" tag -l v0.2.0)"
}
refused() { # LABEL NEEDLE
  expect_fail "$1" "$2" tag_it
  no_tag_anywhere "$1"
}

echo "== ci must have passed in a push run on main for HEAD"
rm -f "$STUB_DIR/runs.json"
refused "no ci run" "no successful push ci run"
run "$head" push main failure | set_runs
refused "a failed run" "no successful push ci run"
run "$head" push main null | set_runs
refused "a run in progress" "no successful push ci run"
run "$head" pull_request main success | set_runs
refused "a pull request run" "no successful push ci run"
run "$head" push side success | set_runs
refused "a push run on another branch" "no successful push ci run"
run 0123456789abcdef0123456789abcdef01234567 push main success | set_runs
refused "a green run of another commit" "no successful push ci run"

echo "== HEAD must be main on GitHub"
green_run
STUB_MAIN_SHA=0123456789abcdef0123456789abcdef01234567 refused "main moved on" "push main first"
echo next >next.txt
git add next.txt
git commit -q -m "feat: not pushed"
refused "an unpushed commit" "push main first"
git reset -q --hard HEAD~1

echo "== the version is never reused"
set_refs refs/heads/main refs/tags/v0.2.0
refused "the tag exists on GitHub" "exists on GitHub"
git tag v0.2.0 HEAD
expect_fail "the tag exists locally" "exists locally" tag_it
git tag -d v0.2.0 >/dev/null
no_tag_anywhere "the tag exists locally"

echo "== the checkout and the pinned version"
set_refs refs/heads/main refs/tags/v0.2.0-rc.1 refs/tags/v0.2.01
echo dirt >dirt.txt
refused "an untracked file" "not clean"
rm dirt.txt
git checkout -q -b side
refused "another branch" "not on main"
git checkout -q main
sed -i 's/^DESKHAVEN_TAG=.*/DESKHAVEN_TAG=0.1.0/' deploy/compose/.env.example
git commit -q -a -m "test: stale tag"
refused "compose pins another version" "does not pin 0.2.0"
git reset -q --hard HEAD~1
printf 'DESKHAVEN_TAG=0.2.0\n' >>deploy/compose/.env.example
git commit -q -a -m "test: two tags"
refused "two DESKHAVEN_TAG lines" "exactly one DESKHAVEN_TAG"
git reset -q --hard HEAD~1

echo "== a release or draft for the version"
set_releases v0.1.0:published v0.2.0:draft
refused "a draft exists" "release or draft for v0.2.0 exists"
set_releases v0.2.0:published
refused "a release exists" "release or draft for v0.2.0 exists"
set_releases v0.1.0:published v0.2.0-rc.1:prerelease

echo "== a version release.yml would refuse"
printf '[workspace.package]\nversion = "0.2.0+build.5"\n' >Cargo.toml
printf 'DESKHAVEN_TAG=0.2.0+build.5\n' >deploy/compose/.env.example
git commit -q -a -m "test: build metadata"
expect_fail "build metadata" "is not X.Y.Z[-prerelease]" tag_it
assert_eq "build metadata: no tag" "" "$(git tag -l 'v0.2.0*')"
git reset -q --hard HEAD~1

echo "== GitHub must answer"
STUB_FAIL=main refused "main cannot be read" "cannot read main of ChanningHe/deskhaven"
STUB_FAIL=runs refused "the ci runs cannot be read" "cannot read the ci runs"
STUB_FAIL=refs refused "the tags cannot be read" "cannot read the tags"
STUB_API_ERROR=1 refused "the releases cannot be read" "cannot read the releases"

echo "== the repository comes from origin"
git remote set-url origin git@github.com:Someone/else.git
STUB_FAIL=main DESKHAVEN_REPO="" refused "an ssh origin" "cannot read main of Someone/else"
git remote set-url origin https://github.com/Someone/else.git
STUB_FAIL=main DESKHAVEN_REPO="" refused "an https origin" "cannot read main of Someone/else"
git remote set-url origin "$origin"
git remote set-url --push origin git@github.com:Pushed/here.git
STUB_FAIL=main DESKHAVEN_REPO="" refused "the push URL, not the fetch URL" "cannot read main of Pushed/here"
git config --unset remote.origin.pushurl
DESKHAVEN_REPO="" refused "a local origin" "cannot tell the GitHub repository"

echo "== a failed push leaves no tag"
git remote set-url origin "$TMP/missing.git"
refused "the push fails" "local tag was deleted again"
git remote set-url origin "$origin"

echo "== all gates pass (refs list v0.2.0-rc.1 and v0.2.01, which only prefix-match)"
{
  run "$head" pull_request main failure
  run "$head" push main success
} | set_runs
expect_pass "tags and pushes v0.2.0" tag_it
assert_eq "an annotated tag" "tag" "$(git cat-file -t v0.2.0)"
assert_eq "the local tag is on HEAD" "$head" "$(git rev-parse 'v0.2.0^{commit}')"
assert_eq "origin has the tag on HEAD" "$head" "$(git --git-dir="$origin" rev-parse 'v0.2.0^{commit}')"
expect_fail "a second run" "exists locally" tag_it

finish
