#!/usr/bin/env bash
# Runs every release script test. Needs bash, git, jq, python3 (3.11+, tomllib) and awk.
# The CI job lints the scripts separately. No network.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
chmod +x "$here"/stubs/* 2>/dev/null || true
status=0
for t in "$here"/test-*.sh; do
  echo
  echo "######## $(basename "$t")"
  bash "$t" || status=1
done
echo
if [ "$status" = 0 ]; then echo "ALL RELEASE SCRIPT TESTS PASSED"; else echo "RELEASE SCRIPT TESTS FAILED"; fi
exit "$status"
