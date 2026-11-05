#!/usr/bin/env bash
# The preflight job's script (extracted from release.yml) against a stubbed gh.
set -euo pipefail
# shellcheck source=lib.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
use_stubs
export STUB_DIR="$TMP/stub"
mkdir -p "$STUB_DIR"

step="$TMP/preflight.sh"
extract_step "no release or draft for this tag" >"$step"
[ -s "$step" ] || { echo "FAIL  the preflight step was not found in $WORKFLOW"; exit 1; }

preflight() { GITHUB_REPOSITORY=ChanningHe/deskhaven GITHUB_REF_NAME="$1" bash "$step"; }

set_releases v0.2.0:draft v0.1.0:published v0.3.0-rc.1:prerelease # the draft is listed for write tokens
expect_fail "a published release exists" "already exists" preflight v0.1.0
expect_fail "a draft exists" "already exists" preflight v0.2.0
expect_fail "a prerelease exists" "already exists" preflight v0.3.0-rc.1
expect_pass "an unrelated tag is free" preflight v0.3.0
expect_pass "v0.2 is not v0.2.0 (exact match)" preflight v0.2
set_releases
expect_pass "no releases at all is free" preflight v0.1.0
expect_fail "an API error fails, it is not 'free'" "HTTP 500" env STUB_API_ERROR=1 bash -c "GITHUB_REPOSITORY=ChanningHe/deskhaven GITHUB_REF_NAME=v0.1.0 bash '$step'"
finish
