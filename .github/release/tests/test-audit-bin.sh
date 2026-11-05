#!/usr/bin/env bash
# audit-bin.sh against a stubbed cargo-audit: enforce and report, comments in deny.toml,
# unmaintained findings, a skipped yanked check, an audit.toml that must not be read.
# Needs python3 (tomllib), as the script itself does.
set -euo pipefail
# shellcheck source=lib.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
python3 -I -c 'import tomllib' || { echo "FAIL  python3 with tomllib (3.11+) is required"; exit 1; }
use_stubs
export STUB_AUDIT_LOG="$TMP/audit.log"
binary="$TMP/binary"
: >"$binary"
report="$TMP/report.txt"

deny="$TMP/deny.toml"
cat >"$deny" <<'EOF'
[advisories]
version = 2
ignore = [
    { id = "RUSTSEC-2023-0071", reason = "rsa: public keys only" },
    # { id = "RUSTSEC-2025-0040", reason = "commented out: must NOT be ignored" },
    { id = "RUSTSEC-2021-0139", reason = "ansi_term unmaintained; mentions RUSTSEC-2024-0375 in its reason" },
]
EOF
audit() { # MODE [ENV=VAL...]
  local mode="$1"
  shift
  env AUDIT_DENY_TOML="$deny" CARGO_TERM_COLOR=always "$@" bash "$RELEASE_DIR/audit-bin.sh" "$mode" "$binary" "$report" "test binary"
}
VULN=vuln:rsa:0.9.10:RUSTSEC-2023-0071
UNMAINT=warn:ansi_term:0.12.1:RUSTSEC-2021-0139

echo "== enforce"
expect_pass "findings that deny.toml ignores pass" audit enforce STUB_FINDINGS="$VULN $UNMAINT"
assert_contains "the audit ran with colour off" "--color never" "$(cat "$STUB_AUDIT_LOG")"
assert_contains "enforce denies warnings" "--deny warnings" "$(cat "$STUB_AUDIT_LOG")"
assert_contains "ignores come from the TOML" "--ignore RUSTSEC-2023-0071" "$(cat "$STUB_AUDIT_LOG")"
assert_not_contains "a commented-out ignore is not passed" "RUSTSEC-2025-0040" "$(cat "$STUB_AUDIT_LOG")"
assert_not_contains "an id mentioned inside a reason is not passed" "RUSTSEC-2024-0375" "$(cat "$STUB_AUDIT_LOG")"
expect_fail "a vulnerability whose ignore is commented out" "RUSTSEC-2025-0040" audit enforce STUB_FINDINGS="$VULN vuln:users:0.11.0:RUSTSEC-2025-0040"
expect_fail "an unmaintained crate that is not ignored" "RUSTSEC-2024-0375" audit enforce STUB_FINDINGS="$VULN warn:atty:0.2.14:RUSTSEC-2024-0375"
expect_fail "a vulnerability that is not ignored names crate and id" "rsa 0.9.10 RUSTSEC-2023-0071" env AUDIT_DENY_TOML="$TMP/empty.toml" STUB_FINDINGS="$VULN" bash -c "printf '[advisories]\nignore = []\n' >'$TMP/empty.toml'; bash '$RELEASE_DIR/audit-bin.sh' enforce '$binary' '$report'"
expect_fail "a broken deny.toml fails loudly" "TOMLDecodeError" env AUDIT_DENY_TOML="$TMP/broken.toml" bash -c "printf '[advisories\nignore = [' >'$TMP/broken.toml'; bash '$RELEASE_DIR/audit-bin.sh' enforce '$binary' '$report'"

echo "== the audit must have looked"
expect_fail "yanked check skipped (index not updated), enforce" "yanked check did not run" audit enforce STUB_AUDIT=noindex
expect_fail "yanked check warning, enforce" "yanked check did not run" audit enforce STUB_AUDIT=indexwarn
expect_pass "yanked check skipped does not block report mode" audit report STUB_AUDIT=noindex
expect_fail "no auditable data, enforce" "carries no cargo-auditable data" audit enforce STUB_AUDIT=nodata
expect_fail "no auditable data, report" "carries no cargo-auditable data" audit report STUB_AUDIT=nodata
expect_fail "advisory database not loaded" "advisory database was not loaded" audit enforce STUB_AUDIT=nodb

echo "== report mode"
expect_pass "findings never fail report mode" audit report STUB_FINDINGS="$VULN $UNMAINT vuln:tungstenite:0.17.2:RUSTSEC-2099-0001"
assert_contains "the report lists every finding" "RUSTSEC-2099-0001" "$(cat "$report")"
assert_not_contains "report mode ignores nothing" "--ignore" "$(cat "$STUB_AUDIT_LOG")"

echo "== an audit.toml must not be read"
mkdir -p "$TMP/hostile/.cargo" "$TMP/hostile-home"
printf '[advisories]\nignore = ["RUSTSEC-2023-0071"]\n' >"$TMP/hostile/.cargo/audit.toml"
printf '[advisories]\nignore = ["RUSTSEC-2023-0071"]\n' >"$TMP/hostile-home/audit.toml"
(cd "$TMP/hostile" && CARGO_HOME="$TMP/hostile-home" audit enforce STUB_FINDINGS="$UNMAINT") >/dev/null 2>&1 || true
log="$(cat "$STUB_AUDIT_LOG")"
assert_contains "no audit.toml visible in the working directory" "audit_toml_in_cwd=no" "$log"
assert_contains "no audit.toml visible in CARGO_HOME" "audit_toml_in_cargo_home=no" "$log"
assert_not_contains "not run from the caller's directory" "cwd=$TMP/hostile" "$log"
assert_not_contains "not run with the caller's CARGO_HOME" "cargo_home=$TMP/hostile-home" "$log"
assert_contains "the binary is given by absolute path" "file=$binary" "$log"
finish
