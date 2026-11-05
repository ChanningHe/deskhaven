#!/usr/bin/env bash
# Shared helpers of the release script tests (sourced). No network: gh, docker, git
# ls-remote and cargo-audit are stubs from tests/stubs, put first on PATH by use_stubs.
#
# RELEASE_DIR points the tests at another copy of the scripts (used to prove that a test
# fails against a deliberately broken script); it defaults to the repository's.

TESTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$TESTS_DIR/../../.." && pwd)"
RELEASE_DIR="${RELEASE_DIR:-$ROOT/.github/release}"
STUBS_DIR="$TESTS_DIR/stubs"
WORKFLOW="${WORKFLOW:-$ROOT/.github/workflows/release.yml}"

TEST_PASSES=0
TEST_FAILS=0
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

ok() { TEST_PASSES=$((TEST_PASSES + 1)); echo "ok    $1"; }
bad() { TEST_FAILS=$((TEST_FAILS + 1)); echo "FAIL  $1"; }

# assert_eq LABEL EXPECTED ACTUAL
assert_eq() {
  if [ "$2" = "$3" ]; then ok "$1"; else bad "$1: expected '$2', got '$3'"; fi
}

# assert_contains LABEL NEEDLE HAYSTACK (fixed string)
assert_contains() {
  if grep -qF -- "$2" <<<"$3"; then ok "$1"; else bad "$1: '$2' not found in: $3"; fi
}

# assert_not_contains LABEL NEEDLE HAYSTACK
assert_not_contains() {
  if grep -qF -- "$2" <<<"$3"; then bad "$1: '$2' unexpectedly found in: $3"; else ok "$1"; fi
}

# run_capture VAR_PREFIX COMMAND...: runs the command, stores stdout+stderr in
# ${VAR_PREFIX}_out and the exit status in ${VAR_PREFIX}_rc.
run_capture() {
  local prefix="$1" out rc
  shift
  set +e
  out="$("$@" 2>&1)"
  rc=$?
  set -e
  printf -v "${prefix}_out" '%s' "$out"
  printf -v "${prefix}_rc" '%s' "$rc"
}

# expect_pass LABEL COMMAND... / expect_fail LABEL NEEDLE COMMAND...
expect_pass() {
  local label="$1"
  shift
  run_capture last "$@"
  if [ "$last_rc" = 0 ]; then ok "$label"; else bad "$label: exit $last_rc: $last_out"; fi
}
expect_fail() {
  local label="$1" needle="$2"
  shift 2
  run_capture last "$@"
  if [ "$last_rc" != 0 ] && grep -qF -- "$needle" <<<"$last_out"; then
    ok "$label"
  else
    bad "$label: exit $last_rc, wanted failure mentioning '$needle': $last_out"
  fi
}

use_stubs() { export PATH="$STUBS_DIR:$PATH"; }

# extract_step STEPNAME: the `run: |` script of the workflow step with that name.
extract_step() {
  awk -v name="$1" '
    $0 == "      - name: " name { f = 1; next }
    f && /run: \|/ { r = 1; next }
    r { if ($0 ~ /^          /) { sub(/^          /, ""); print } else if ($0 !~ /^$/) exit }' "$WORKFLOW"
}

# digest_of_text TEXT: a deterministic well-formed sha256 digest for fixtures.
fixture_digest() { printf 'sha256:%s' "$(printf '%s' "$1" | sha256sum | cut -d' ' -f1)"; }

finish() {
  echo "-- $(basename "$0"): ${TEST_PASSES} passed, ${TEST_FAILS} failed"
  [ "$TEST_FAILS" = 0 ]
}
