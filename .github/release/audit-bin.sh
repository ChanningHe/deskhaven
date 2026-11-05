#!/usr/bin/env bash
# Audits a release binary built with `cargo auditable` against the RustSec advisory
# database (`cargo audit bin`).
#
#   enforce  exit 1 on any vulnerability that deny.toml does not ignore (deskhaven)
#   report   never fails on vulnerabilities, only prints and records them (hbbr, whose
#            upstream lockfile has advisories we cannot fix without modifying upstream)
#
# Both modes fail if the binary carries no dependency data or the advisory database could
# not be loaded: a pass must mean "audited", not "could not look".
#
# Ignored advisories come from the [advisories] ignore list of deny.toml, the one place
# where accepted advisories (with their reasons) are recorded. In report mode nothing is
# ignored: the report shows everything.
#
# Usage: .github/release/audit-bin.sh MODE BINARY REPORT_FILE [TITLE]
# Needs: cargo-audit (`.github/ci-setup.sh cargo-audit@VERSION`). When GITHUB_STEP_SUMMARY
# is set, the result is appended to the job summary.
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

ignore_args=()
if [ "$mode" = enforce ]; then
  while IFS= read -r id; do
    ignore_args+=(--ignore "$id")
  done < <(sed -nE 's/.*[{ ]id = "(RUSTSEC-[0-9]{4}-[0-9]{4})".*/\1/p' "$root/deny.toml")
  if [ "${#ignore_args[@]}" -eq 0 ]; then
    echo "audit-bin.sh: no ignored advisories found in deny.toml (parse error?)" >&2
    exit 2
  fi
fi

status=0
# Global options (--ignore) go before the `bin` subcommand.
cargo audit "${ignore_args[@]}" bin "$binary" >"$report" 2>&1 || status=$?
cat "$report"

grep -q "Found 'cargo auditable' data" "$report" || {
  echo "audit-bin.sh: $binary carries no cargo-auditable data (built without cargo auditable?)" >&2
  exit 1
}
grep -q "security advisories" "$report" || {
  echo "audit-bin.sh: the advisory database was not loaded" >&2
  exit 1
}

vulnerabilities="$(sed -nE 's/^error: ([0-9]+) vulnerabilit.*/\1/p' "$report")"
vulnerabilities="${vulnerabilities:-0}"

if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  {
    echo "### cargo audit: $title ($mode)"
    echo
    echo "- vulnerabilities: **$vulnerabilities**"
    echo "- advisories ignored through deny.toml: $((${#ignore_args[@]} / 2))"
    echo
    echo '```'
    cat "$report"
    echo '```'
  } >>"$GITHUB_STEP_SUMMARY"
fi

if [ "$status" -ne 0 ] && [ "$vulnerabilities" -eq 0 ]; then
  echo "audit-bin.sh: cargo audit failed without reporting vulnerabilities (exit $status)" >&2
  exit 1
fi
if [ "$mode" = enforce ] && [ "$vulnerabilities" -gt 0 ]; then
  echo "audit-bin.sh: $vulnerabilities vulnerabilities in $title are not ignored in deny.toml" >&2
  exit 1
fi
exit 0
