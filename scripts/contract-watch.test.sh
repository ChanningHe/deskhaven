#!/usr/bin/env bash
# Regression checks for scripts/contract-watch.sh against local fake upstreams (file://),
# no network. Usage: scripts/contract-watch.test.sh
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SCRIPT=${CONTRACT_WATCH_SCRIPT:-$HERE/contract-watch.sh}
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# The watch list is read from the script itself, so there is still only one copy.
eval "$(sed -n '/^WATCHED=(/,/^)/p' "$SCRIPT")"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}
pass() { echo "PASS: $*"; }

git_in() { git -C "$1" -c user.name=t -c user.email=t@example.invalid -c commit.gpgsign=false "${@:2}"; }

new_repo() { # dir
  git init -q -b master "$1"
  git_in "$1" config uploadpack.allowFilter true
  git_in "$1" config uploadpack.allowAnySHA1InWant true
}

# CJK text: 3 bytes per character, so a byte cut lands inside a character.
cjk_text() { # line-count
  local i
  for ((i = 0; i < $1; i++)); do echo "漢字漢字漢字漢字漢字漢字漢字漢字漢字漢字漢字漢字漢字漢字漢字漢字漢字漢字漢字漢字 $i"; done
}

# make_fake <name> [skipped-path]: fake client + hbb_common, base and head commits.
# Head changes: a large CJK addition to src/common.rs, a context line made of exactly three
# backticks in src/core_main.rs, and a new hbb_common commit touching rendezvous.proto.
# Sets FAKE_BASE (client base sha).
make_fake() {
  local root=$TMP/$1 skip=${2:-} entry path file hbb_base hbb_head
  new_repo "$root/hbb"
  mkdir -p "$root/hbb/protos"
  echo "message A {}" >"$root/hbb/protos/rendezvous.proto"
  git_in "$root/hbb" add -A
  git_in "$root/hbb" commit -q -m base
  hbb_base=$(git_in "$root/hbb" rev-parse HEAD)
  echo "message A2 {}" >>"$root/hbb/protos/rendezvous.proto"
  git_in "$root/hbb" commit -q -a -m head
  hbb_head=$(git_in "$root/hbb" rev-parse HEAD)

  new_repo "$root/client"
  for entry in "${WATCHED[@]}"; do
    path=${entry%%|*}
    [[ $path == hbb_common/* ]] && continue
    [ "$path" = "$skip" ] && continue
    file=$root/client/${path//\*/x}
    mkdir -p "$(dirname "$file")"
    printf 'first\n```\nlast\n' >"$file"
  done
  git_in "$root/client" add -A
  git_in "$root/client" update-index --add --cacheinfo "160000,$hbb_base,libs/hbb_common"
  git_in "$root/client" commit -q -m base
  FAKE_BASE=$(git_in "$root/client" rev-parse HEAD)
  cjk_text 3000 >>"$root/client/src/common.rs"
  printf 'FIRST\n```\nlast\n' >"$root/client/src/core_main.rs"
  git_in "$root/client" add -A
  git_in "$root/client" update-index --add --cacheinfo "160000,$hbb_head,libs/hbb_common"
  git_in "$root/client" commit -q -m head
}

# run_watch <name> [env assignments...]: runs the script in dry-run, sets STATUS and BODY.
run_watch() {
  local name=$1
  shift
  STATUS=0
  BODY=$TMP/$name.body
  env UPSTREAM_URL="file://$TMP/$name/client" HBB_COMMON_URL="file://$TMP/$name/hbb" "$@" \
    "$SCRIPT" --dry-run --base "$FAKE_BASE" >"$BODY" 2>"$TMP/$name.err" || STATUS=$?
}

make_fake ok
# A cap of N bytes cuts inside a 3-byte character for two of any three consecutive N.
for budget in 40000 40001 40002; do
  run_watch ok CONTRACT_WATCH_DIFF_BUDGET_BYTES=$budget
  [ "$STATUS" -eq 0 ] || fail "budget $budget: exit $STATUS: $(cat "$TMP/ok.err")"
  [ -s "$BODY" ] || fail "budget $budget: empty body"
  iconv -f UTF-8 -t UTF-8 <"$BODY" >/dev/null || fail "budget $budget: body is not valid UTF-8"
  grep -q '^_Truncated:' "$BODY" || fail "budget $budget: expected a truncated diff"
done
pass "CJK diff truncated mid-character still yields a valid, non-empty body (budgets 40000-40002)"

# A context line of exactly three backticks must not close the (longer) fence.
grep -qx ' ```' "$BODY" || fail "fixture lost its three-backtick context line"
grep -qx '````diff' "$BODY" || fail "diff block is not fenced with 4 backticks"
pass "fence is longer than the backtick run inside the diff"

# Body clipped to a tiny limit: head -c stops early, no SIGPIPE, still exit 0.
run_watch ok CONTRACT_WATCH_BODY_LIMIT_BYTES=3000
[ "$STATUS" -eq 0 ] || fail "tiny body limit: exit $STATUS: $(cat "$TMP/ok.err")"
[ "$(wc -c <"$BODY")" -le 3000 ] || fail "body exceeds the tiny limit"
[ -s "$BODY" ] || fail "tiny body limit: empty body"
pass "body larger than the limit is clipped without SIGPIPE"

# A watched entry that exists at neither commit must fail loudly.
make_fake missing src/custom_server.rs
run_watch missing
[ "$STATUS" -ne 0 ] || fail "missing watched file: expected a failure"
grep -q "watched entry 'src/custom_server.rs'" "$TMP/missing.err" || fail "missing entry not named: $(cat "$TMP/missing.err")"
pass "watched entry matching nothing fails and is named"
