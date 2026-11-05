#!/usr/bin/env bash
# tags.sh: plan, highest and prev, with the semver edge cases (long numbers, rc ordering,
# old-line patches). All tag lists come from stdin, as in the workflow.
set -euo pipefail
# shellcheck source=lib.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

tags() { "$RELEASE_DIR/tags.sh" "$@"; }
list() { printf '%s\n' "$@"; }
plan() { local t="$1"; shift; tags plan "$t" < <(list "$@") | tr '\n' ' '; }
prev() { local t="$1"; shift; tags prev "$t" < <(list "$@"); }
highest() { local line="$1"; shift; tags highest ${line:+"$line"} < <(list "$@"); }

echo "== plan"
assert_eq "first release" "latest=true minor_latest=true " "$(plan v0.1.0)"
assert_eq "newest of everything" "latest=true minor_latest=true " "$(plan v0.10.0 v0.1.0 v0.2.0)"
assert_eq "0.10 is above 0.2" "latest=false minor_latest=true " "$(plan v0.2.0 v0.1.0 v0.10.0)"
assert_eq "old-line patch" "latest=false minor_latest=true " "$(plan v0.1.1 v0.1.0 v0.2.0)"
assert_eq "superseded in its line" "latest=false minor_latest=false " "$(plan v0.1.0 v0.1.1 v0.2.0)"
assert_eq "prerelease" "latest=false minor_latest=false " "$(plan v0.2.0-rc.2 v0.1.0)"
assert_eq "11-digit patch is above a 2-digit one" "latest=true minor_latest=true " "$(plan v1.0.12345678901 v1.0.99)"

echo "== highest"
assert_eq "highest overall" "v0.10.0" "$(highest '' v0.2.0 v0.10.0 v0.9.9 v0.11.0-rc.1)"
assert_eq "highest in the 0.2 line" "v0.2.5" "$(highest 0.2 v0.2.0 v0.2.5 v0.3.0 v0.2.4)"
assert_eq "no final release" "" "$(highest '' v0.1.0-rc.1)"

echo "== prev (published releases on stdin; final releases skip prereleases)"
published=(v0.1.0 v0.1.1 v0.2.0-rc.1 v0.2.0-rc.2 v0.2.0-rc.10 v0.2.0 v0.10.0 vfoo)
assert_eq "final skips the rcs" "v0.1.1" "$(prev v0.2.0 "${published[@]}")"
assert_eq "first rc" "v0.1.1" "$(prev v0.2.0-rc.1 "${published[@]}")"
assert_eq "rc.2" "v0.2.0-rc.1" "$(prev v0.2.0-rc.2 "${published[@]}")"
assert_eq "rc.10 sorts after rc.2" "v0.2.0-rc.2" "$(prev v0.2.0-rc.10 "${published[@]}")"
assert_eq "old-line patch" "v0.1.0" "$(prev v0.1.1 "${published[@]}")"
assert_eq "0.10.0 follows 0.2.0" "v0.2.0" "$(prev v0.10.0 "${published[@]}")"
assert_eq "the first release has none" "" "$(prev v0.1.0 "${published[@]}")"
assert_eq "an unpublished hypothetical tag" "v0.10.0" "$(prev v0.11.0 "${published[@]}")"
long=(v1.0.0 v1.0.9 v1.0.10 v1.0.99999999999 v1.0.100000000000 v1.0.100000000001-rc.1 v1.0.100000000001 v1.0.0-rc.5 v1.0.0-rc.12345678901 v1.0.0-rc.99999999999)
assert_eq "12 digits follow 11 digits" "v1.0.99999999999" "$(prev v1.0.100000000000 "${long[@]}")"
assert_eq "11 digits follow 2 digits" "v1.0.10" "$(prev v1.0.99999999999 "${long[@]}")"
assert_eq "10 follows 9" "v1.0.9" "$(prev v1.0.10 "${long[@]}")"
assert_eq "final skips its 12-digit rc" "v1.0.100000000000" "$(prev v1.0.100000000001 "${long[@]}")"
assert_eq "numeric prerelease ids with 11 digits" "v1.0.0-rc.5" "$(prev v1.0.0-rc.12345678901 "${long[@]}")"
assert_eq "...and 11 digits again" "v1.0.0-rc.12345678901" "$(prev v1.0.0-rc.99999999999 "${long[@]}")"
assert_eq "a final has no earlier final" "" "$(prev v1.0.0 "${long[@]}")"
expect_fail "a tag that is not semver is rejected" "is not vX.Y.Z" tags prev nonsense </dev/null
finish
