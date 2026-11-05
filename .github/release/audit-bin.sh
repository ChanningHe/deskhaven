#!/usr/bin/env bash
# Audits a release binary built with `cargo auditable` against the RustSec advisory
# database (`cargo audit bin`).
#
#   enforce  exit 1 on any vulnerability, unmaintained, unsound or yanked finding that
#            deny.toml does not ignore (deskhaven)
#   report   never fails on findings, only prints and records them (hbbr, whose upstream
#            lockfile has advisories we cannot fix without modifying upstream); nothing
#            is ignored, the report shows everything
#
# Both modes fail if the binary carries no dependency data or the advisory database could
# not be loaded: a pass must mean "audited", not "could not look".
#
# Ignored advisories are the `advisories.ignore` ids of deny.toml, read with a real TOML
# parser (comments and look-alike text do not count), the one place where accepted
# advisories are recorded with their reasons. AUDIT_DENY_TOML overrides the file (tests).
#
# Colour is forced off: cargo-audit honours CARGO_TERM_COLOR=always, and escape codes
# would break the greps below.
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
  # (A deny.toml that does not parse fails in python above; an empty ignore list is legal.)
  # Unmaintained, unsound and yanked findings fail too, unless ignored above.
  audit_args+=(--deny warnings)
fi

status=0
cargo audit "${audit_args[@]}" bin "$binary" >"$report" 2>&1 || status=$?
cat "$report"

grep -q "Found 'cargo auditable' data" "$report" || {
  echo "audit-bin.sh: $binary carries no cargo-auditable data (built without cargo auditable?)" >&2
  exit 1
}
grep -q "security advisories" "$report" || {
  echo "audit-bin.sh: the advisory database was not loaded" >&2
  exit 1
}

# cargo-audit only WARNS ("couldn't update/open crates.io index", "couldn't check if the
# package is yanked") when the index is unreachable and then skips the yanked check: that
# must not read as "no yanked crates" when enforcing.
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
