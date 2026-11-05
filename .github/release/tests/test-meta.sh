#!/usr/bin/env bash
# meta.sh: every publish and refusal case, in a scratch repository with a stubbed gh.
set -euo pipefail
# shellcheck source=lib.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
use_stubs
export STUB_DIR="$TMP/stub"
mkdir -p "$STUB_DIR"

repo="$TMP/repo"
mkdir -p "$repo/.github/release" "$repo/deploy/compose"
cp "$RELEASE_DIR/meta.sh" "$repo/.github/release/"
cd "$repo"
git init -q .
git config user.email t@example.com
git config user.name t
set_version() { # cargo version, compose tag
  printf '[workspace.package]\nversion = "%s"\nedition = "2024"\n\n[profile.release]\nlto = "thin"\n' "$1" >Cargo.toml
  printf '# header\nDESKHAVEN_TAG=%s\nOTHER=1\n' "$2" >deploy/compose/.env.example
}
set_version 0.1.0 0.1.0
git add -A
git commit -q -m "feat: base"
base_sha="$(git rev-parse HEAD)"
git update-ref refs/remotes/origin/main HEAD
git checkout -q -b side
echo x >side.txt
git add side.txt
git commit -q -m "feat: not on main"
side_sha="$(git rev-parse HEAD)"
git checkout -q -

# meta EVENT REFTYPE REFNAME SHA [ENV=VAL...]: runs meta.sh, outputs in $out_file
out_file="$TMP/github-output"
meta() {
  local event="$1" reftype="$2" refname="$3" sha="$4"
  shift 4
  : >"$out_file"
  env GITHUB_OUTPUT="$out_file" GITHUB_EVENT_NAME="$event" GITHUB_REF_TYPE="$reftype" GITHUB_REF_NAME="$refname" \
    GITHUB_SHA="$sha" GITHUB_REPOSITORY=ChanningHe/deskhaven GITHUB_REPOSITORY_OWNER=ChanningHe "$@" \
    bash "$repo/.github/release/meta.sh"
}
published() { tr '\n' ' ' <"$out_file"; }

echo "== trigger matrix"
run_capture m meta push tag v0.1.0 "$base_sha" STUB_IMAGE=absent
assert_eq "push tag v0.1.0 publishes" "0" "$m_rc"
assert_contains "push tag outputs publish=true" "publish=true" "$(published)"
run_capture m meta workflow_dispatch tag v0.1.0 "$base_sha"
assert_contains "workflow_dispatch on the tag never publishes" "publish=false" "$(published)"
run_capture m meta workflow_dispatch tag v0.1.0 "$base_sha" STUB_IMAGE=exists
assert_eq "dispatch ignores existing images (dry run)" "0" "$m_rc"
assert_contains "dispatch on the tag, image exists: still publish=false" "publish=false" "$(published)"
run_capture m meta workflow_dispatch branch main "$base_sha"
assert_contains "workflow_dispatch on a branch never publishes" "publish=false" "$(published)"
run_capture m meta push branch main "$base_sha"
assert_contains "push to a branch does not publish" "publish=false" "$(published)"

echo "== refusals when publishing"
expect_fail "tag that is not semver" "is not vX.Y.Z" meta push tag vfoo "$base_sha" STUB_IMAGE=absent
expect_fail "tag differs from the workspace version" "does not match the workspace version" meta push tag v0.1.1 "$base_sha" STUB_IMAGE=absent
expect_fail "image tag already exists" "already exists; an image tag is never moved" meta push tag v0.1.0 "$base_sha" STUB_IMAGE=exists
run_capture m meta push tag v0.1.0 "$base_sha" STUB_IMAGE=other
assert_eq "package without this tag is free" "0" "$m_rc"
expect_fail "API 403 is not 'free'" "cannot tell whether" meta push tag v0.1.0 "$base_sha" STUB_IMAGE=denied
expect_fail "network error is not 'free'" "cannot tell whether" meta push tag v0.1.0 "$base_sha" STUB_IMAGE=neterr
expect_fail "commit not on main" "is not on main" meta push tag v0.1.0 "$side_sha" STUB_IMAGE=absent
set_version 0.1.0 0.0.9
expect_fail ".env.example pins another tag" "DESKHAVEN_TAG=0.1.0" meta push tag v0.1.0 "$base_sha" STUB_IMAGE=absent
printf 'DESKHAVEN_TAG=0.1.0\nDESKHAVEN_TAG=0.0.9\n' >deploy/compose/.env.example
expect_fail ".env.example with two DESKHAVEN_TAG lines" "exactly one DESKHAVEN_TAG" meta push tag v0.1.0 "$base_sha" STUB_IMAGE=absent

echo "== prerelease"
set_version 0.1.0-rc.1 0.1.0-rc.1
run_capture m meta push tag v0.1.0-rc.1 "$base_sha" STUB_IMAGE=absent
assert_contains "rc tag is a prerelease" "prerelease=true" "$(published)"
finish
