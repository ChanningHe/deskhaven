#!/usr/bin/env bash
#   enforce  exit 1 on any finding deny.toml does not ignore
#   report   never fails on findings (hbbr: upstream's lockfile is not ours to fix)
# Both modes fail without dependency data or advisory database: a pass must mean "audited".
# AUDIT_DENY_TOML overrides deny.toml. Colour is off: escape codes would break the greps.
#
# Usage: .github/release/audit-bin.sh MODE BINARY REPORT_FILE [TITLE]
# Needs: cargo-audit (`.github/ci-setup.sh cargo-audit@VERSION`) and python3 (tomllib).
# When GITHUB_STEP_SUMMARY is set, the result is appended to the job summary.
set -euo pipefail

mode="${1:?usage: audit-bin.sh enforce|report BINARY REPORT_FILE [TITLE]}"
binary="${2:?missing BINARY}"
report="${3:?missing REPORT_FILE}"
title="${4:-$(basename "$binary")}"

case "$mode" in
  enforce | report) ;;
  *)
    echo "audit-bin.sh: mode must be enforce or report, got '$mode'" >&2
    exit 2
    ;;
esac

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
deny_toml="${AUDIT_DENY_TOML:-$root/deny.toml}"

# Global options go before the `bin` subcommand.
audit_args=(--color never)
ignored=0
if [ "$mode" = enforce ]; then
  ids="$(python3 -I -c '
import sys, tomllib
with open(sys.argv[1], "rb") as f:
    entries = tomllib.load(f).get("advisories", {}).get("ignore", [])
for entry in entries:
    print(entry if isinstance(entry, str) else entry["id"])
' "$deny_toml")"
  while IFS= read -r id; do
    [ -n "$id" ] || continue
    audit_args+=(--ignore "$id")
    ignored=$((ignored + 1))
  done <<<"$ids"
  # Unmaintained, unsound and yanked findings are warnings: fail on them too.
  audit_args+=(--deny warnings)
fi

# An audit.toml in ./.cargo/ or $CARGO_HOME could silently ignore advisories, so cargo-audit
# runs from an empty directory with a fresh CARGO_HOME.
binary="$(realpath "$binary")"
report="$(realpath -m "$report")"
sandbox="$(mktemp -d)"
trap 'rm -rf "$sandbox"' EXIT
mkdir "$sandbox/cwd" "$sandbox/cargo-home"
status=0
(cd "$sandbox/cwd" && CARGO_HOME="$sandbox/cargo-home" cargo-audit audit "${audit_args[@]}" bin "$binary") >"$report" 2>&1 || status=$?
cat "$report"

grep -q "security advisories" "$report" || {
  echo "audit-bin.sh: the advisory database was not loaded" >&2
  exit 1
}
grep -q "Found 'cargo auditable' data" "$report" || {
  echo "audit-bin.sh: $binary carries no cargo-auditable data (built without cargo auditable?)" >&2
  exit 1
}

# An unreachable index only warns and skips the yanked check: that must not pass when enforcing.
if [ "$mode" = enforce ]; then
  if ! grep -q "Updating.*crates.io index" "$report" ||
    grep -qE "couldn't (update|open) crates.io index|couldn't check if the package is yanked" "$report"; then
    echo "audit-bin.sh: the yanked check did not run (crates.io index unavailable); not accepting an unchecked binary" >&2
    exit 1
  fi
fi

vulnerabilities="$(sed -nE 's/^error: ([0-9]+) vulnerabilit.*/\1/p' "$report")"
warnings="$(sed -nE 's/^(warning|error): ([0-9]+) (denied|allowed) warning.*/\2/p' "$report" | head -n 1)"
findings="$(awk '/^Crate:/ {c = $2} /^Version:/ {v = $2} /^ID:/ {print c " " v " " $2}' "$report" | paste -sd, -)"

if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  {
    echo "### cargo audit: $title ($mode)"
    echo
    echo "- vulnerabilities: **${vulnerabilities:-0}**, warnings: **${warnings:-0}**"
    echo "- advisories ignored through deny.toml: $ignored"
    echo
    echo '```'
    cat "$report"
    echo '```'
  } >>"$GITHUB_STEP_SUMMARY"
fi

if [ "$status" -ne 0 ] && [ "$mode" = enforce ]; then
  echo "audit-bin.sh: $title failed cargo audit (exit $status): ${vulnerabilities:-0} vulnerabilities, ${warnings:-0} denied warnings; not ignored in deny.toml: ${findings:-see report}" >&2
  exit 1
fi
if [ "$status" -ne 0 ] && [ -z "$vulnerabilities" ] && [ -z "$warnings" ]; then
  echo "audit-bin.sh: cargo audit failed without reporting findings (exit $status)" >&2
  exit 1
fi
exit 0
