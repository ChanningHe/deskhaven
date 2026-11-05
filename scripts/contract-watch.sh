#!/usr/bin/env bash
# Upstream client contract watch (09-testing §2.3).
#
# Diffs the watched upstream client files between the last verified commit
# (tests/contract/UPSTREAM_COMMIT) and the current upstream master. When anything changed
# it opens a GitHub issue carrying a diff summary and the affected 03 sections; the
# issue is not repeated for the same upstream master sha.
#
# Usage: scripts/contract-watch.sh [--dry-run] [--base <sha>]
#   --dry-run     print the issue body to stdout instead of touching GitHub
#   --base <sha>  compare against this (full) client commit instead of UPSTREAM_COMMIT
# Environment (testing only): UPSTREAM_URL, HBB_COMMON_URL, UPSTREAM_BRANCH.
# Creating the issue needs the gh CLI with a token that may write issues, and GH_REPO
# (owner/name) unless run inside a clone of the target repository.
set -euo pipefail

UPSTREAM_URL=${UPSTREAM_URL:-https://github.com/rustdesk/rustdesk.git}
HBB_COMMON_URL=${HBB_COMMON_URL:-https://github.com/rustdesk/hbb_common.git}
UPSTREAM_BRANCH=${UPSTREAM_BRANCH:-master}

ISSUE_LABEL=contract-drift
# GitHub rejects issue bodies above 65536 characters; stay well below.
BODY_LIMIT_BYTES=60000
DIFF_BUDGET_BYTES=40000
SUBMODULE_PATH=libs/hbb_common
# Watched paths carrying this prefix live in the hbb_common submodule (repo-relative path
# after the prefix), diffed at the submodule commits the two client commits pin.
SUBMODULE_PREFIX=hbb_common/

# THE watched-file list (the only copy): "<git pathspec>|<03 sections it feeds>".
# Pathspec `*` crosses directories. Section "07" is the hbbs fork design doc.
WATCHED=(
  "src/common.rs|1.1 1.2 1.3 1.4 1.5"
  "src/custom_server.rs|1.1"
  "src/hbbs_http/*.rs|1.4 2.1 2.2 2.7 3.3 3.5 3.6"
  "src/ui_interface.rs|2.8"
  "src/core_main.rs|2.9"
  "src/server/connection.rs|2.3 2.5 2.6 07"
  "src/client.rs|5.1"
  "src/rendezvous_mediator.rs|1.4 07"
  "src/ui_session_interface.rs|2.4 5.3"
  "flutter/lib/common/hbbs/hbbs.dart|4 5.1"
  "flutter/lib/models/ab_model.dart|5.1 5.2 5.3"
  "flutter/lib/models/group_model.dart|4"
  "flutter/lib/models/user_model.dart|1.3 3.1 3.2 3.3"
  "flutter/lib/models/model.dart|6"
  "flutter/lib/common/widgets/login.dart|3.2"
  "hbb_common/protos/rendezvous.proto|1.4 07"
  "hbb_common/protos/message.proto|07"
)

die() {
  echo "contract-watch: $*" >&2
  exit 1
}

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
DRY_RUN=0
BASE_OVERRIDE=""
while [ $# -gt 0 ]; do
  case $1 in
    --dry-run) DRY_RUN=1 ;;
    --base)
      [ $# -ge 2 ] || die "--base needs a commit sha"
      BASE_OVERRIDE=$2
      shift
      ;;
    -h | --help)
      sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed '$d'
      exit 0
      ;;
    *) die "unknown argument: $1" ;;
  esac
  shift
done

SHA_RE='^[0-9a-f]{40}$'
BASE_SHA=${BASE_OVERRIDE:-$(tr -d '[:space:]' <"$ROOT/tests/contract/UPSTREAM_COMMIT")}
[[ $BASE_SHA =~ $SHA_RE ]] || die "base commit is not a full 40-char lowercase sha: '$BASE_SHA'"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
export GIT_TERMINAL_PROMPT=0
# One scratch repository per upstream repo, so lazy blob fetches never ask the wrong remote.
CLIENT_REPO=$WORK/client
HBB_REPO=$WORK/hbb_common

# Treeless shallow fetch into a fresh repo: commits and trees only; blobs of watched files
# come on demand when diffing. Fetched content is only read, never checked out or executed.
fetch_commits() { # repo-dir url refspec...
  local dir=$1 url=$2
  shift 2
  git init -q "$dir"
  git -C "$dir" remote add origin "$url"
  git -C "$dir" fetch -q --depth=1 --filter=blob:none origin "$@" ||
    die "fetching from $url failed"
}

fetch_commits "$CLIENT_REPO" "$UPSTREAM_URL" \
  "$BASE_SHA:refs/contract/base" "$UPSTREAM_BRANCH:refs/contract/head"
HEAD_SHA=$(git -C "$CLIENT_REPO" rev-parse refs/contract/head)
SHORT_HEAD=${HEAD_SHA:0:12}
SHORT_BASE=${BASE_SHA:0:12}

if [ "$HEAD_SHA" = "$BASE_SHA" ]; then
  echo "contract-watch: upstream $UPSTREAM_BRANCH is still at the verified commit $SHORT_BASE" >&2
  exit 0
fi

submodule_pin() { # commit
  local pin
  pin=$(git -C "$CLIENT_REPO" ls-tree "$1" -- "$SUBMODULE_PATH" | awk '$1 == "160000" { print $3 }')
  [[ $pin =~ $SHA_RE ]] || die "no $SUBMODULE_PATH submodule pin found at client commit $1"
  echo "$pin"
}
HBB_BASE=$(submodule_pin "$BASE_SHA")
HBB_HEAD=$(submodule_pin "$HEAD_SHA")
if [ "$HBB_BASE" != "$HBB_HEAD" ]; then
  fetch_commits "$HBB_REPO" "$HBB_COMMON_URL" \
    "$HBB_BASE:refs/contract/hbb-base" "$HBB_HEAD:refs/contract/hbb-head"
fi

# entry_diff <path> [git diff options...]: diff of one watched path between base and head.
entry_diff() {
  local path=$1 repo=$CLIENT_REPO from=$BASE_SHA to=$HEAD_SHA prefix=""
  shift
  if [[ $path == "$SUBMODULE_PREFIX"* ]]; then
    # Same pin on both sides: nothing changed, and the submodule was not fetched.
    [ "$HBB_BASE" != "$HBB_HEAD" ] || return 0
    repo=$HBB_REPO from=$HBB_BASE to=$HBB_HEAD prefix=$SUBMODULE_PREFIX path=${path#"$SUBMODULE_PREFIX"}
  fi
  git -C "$repo" diff --no-color --no-ext-diff --no-textconv \
    --src-prefix="a/$prefix" --dst-prefix="b/$prefix" "$@" "$from" "$to" -- "$path"
}

# Truncate stdin to at most $1 bytes without leaving a broken UTF-8 sequence behind.
clip() { head -c "$1" | iconv -c -f UTF-8 -t UTF-8; }

CHANGED=()
for entry in "${WATCHED[@]}"; do
  # Assigned on its own line so a failing git aborts the script (set -e) instead of
  # reading as "no change".
  numstat=$(entry_diff "${entry%%|*}" --numstat)
  if [ -n "$numstat" ]; then
    CHANGED+=("$entry")
  fi
done

if [ ${#CHANGED[@]} -eq 0 ]; then
  echo "contract-watch: $UPSTREAM_BRANCH moved ($SHORT_BASE -> $SHORT_HEAD) but no watched file changed" >&2
  exit 0
fi

FENCE='```' # diff lines always start with a prefix char, so a fence inside a diff cannot close it

# Full diff of every changed entry on disk, then a fair share of the byte budget for each:
# small diffs are shown whole and what they leave over goes to the bigger ones.
DIFF_DIR=$WORK/diffs
mkdir "$DIFF_DIR"
SIZES=()
for i in "${!CHANGED[@]}"; do
  entry_diff "${CHANGED[$i]%%|*}" >"$DIFF_DIR/$i.diff"
  SIZES+=("$(wc -c <"$DIFF_DIR/$i.diff")")
done

CAPS=()
remaining=$DIFF_BUDGET_BYTES
open=${#CHANGED[@]}
for i in "${!CHANGED[@]}"; do CAPS[i]=-1; done
settled_any=1
while [ "$settled_any" -eq 1 ] && [ "$open" -gt 0 ]; do
  settled_any=0
  share=$((remaining / open))
  for i in "${!CHANGED[@]}"; do
    if [ "${CAPS[$i]}" -lt 0 ] && [ "${SIZES[$i]}" -le "$share" ]; then
      CAPS[i]=${SIZES[$i]}
      remaining=$((remaining - SIZES[i]))
      open=$((open - 1))
      settled_any=1
    fi
  done
done
for i in "${!CHANGED[@]}"; do
  if [ "${CAPS[$i]}" -lt 0 ]; then CAPS[i]=$((remaining / open)); fi
done

# "1.1 2.3 07" -> "03 §1.1, §2.3; 07"
section_list() {
  local s refs="" extra=""
  for s in $1; do
    if [ "$s" = 07 ]; then
      extra="07"
    else
      refs+="${refs:+, }§$s"
    fi
  done
  if [ -n "$refs" ] && [ -n "$extra" ]; then
    echo "03 $refs; $extra"
  elif [ -n "$refs" ]; then
    echo "03 $refs"
  else
    echo "$extra"
  fi
}

build_body() {
  local entry path i
  cat <<HEAD
Upstream \`rustdesk/rustdesk\` \`$UPSTREAM_BRANCH\` moved past the last commit the deskhaven client contract was verified against.

| | commit |
|---|---|
| verified (\`tests/contract/UPSTREAM_COMMIT\`) | [\`$BASE_SHA\`](${UPSTREAM_URL%.git}/commit/$BASE_SHA) |
| upstream $UPSTREAM_BRANCH | [\`$HEAD_SHA\`](${UPSTREAM_URL%.git}/commit/$HEAD_SHA) |

Full comparison: ${UPSTREAM_URL%.git}/compare/$BASE_SHA...$HEAD_SHA
HEAD
  if [ "$HBB_BASE" != "$HBB_HEAD" ]; then
    echo
    echo "hbb_common submodule pin: [\`${HBB_BASE:0:12}\`](${HBB_COMMON_URL%.git}/commit/$HBB_BASE) -> [\`${HBB_HEAD:0:12}\`](${HBB_COMMON_URL%.git}/commit/$HBB_HEAD), comparison: ${HBB_COMMON_URL%.git}/compare/$HBB_BASE...$HBB_HEAD"
  fi

  printf '\n## Changed watched files and affected sections\n\n'
  for entry in "${CHANGED[@]}"; do
    echo "- \`${entry%%|*}\`: $(section_list "${entry#*|}")"
  done

  printf '\n## Diff summary\n'
  for entry in "${CHANGED[@]}"; do
    path=${entry%%|*}
    printf '\n%s\n\n%s\n' "\`$path\`" "$FENCE"
    entry_diff "$path" --stat=100
    echo "$FENCE"
  done

  printf '\n## Diff (large files are truncated to fit the issue size limit)\n'
  for i in "${!CHANGED[@]}"; do
    path=${CHANGED[$i]%%|*}
    printf '\n%s\n\n%sdiff\n' "\`$path\`" "$FENCE"
    clip "${CAPS[$i]}" <"$DIFF_DIR/$i.diff"
    if [ "${SIZES[$i]}" -gt "${CAPS[$i]}" ]; then
      printf '\n... truncated (%s of %s bytes shown), see the full comparison above\n' "${CAPS[$i]}" "${SIZES[$i]}"
    fi
    echo "$FENCE"
  done

  cat <<TAIL

## Review checklist

- [ ] Review the diff against the listed sections of 03 and update them where behaviour changed
- [ ] Update \`tests/contract/fixtures\` and the contract tests
- [ ] Set \`tests/contract/UPSTREAM_COMMIT\` to \`$HEAD_SHA\` (or the newer master reviewed), then close this issue

_Opened by \`scripts/contract-watch.sh\` (workflow \`contract-watch\`). Not repeated for the same upstream sha._
TAIL
}

BODY_FILE=$WORK/body.md
build_body | clip "$BODY_LIMIT_BYTES" >"$BODY_FILE"

if [ "$DRY_RUN" -eq 1 ]; then
  cat "$BODY_FILE"
  exit 0
fi

command -v gh >/dev/null || die "the gh CLI is required to open the issue (use --dry-run to only print the body)"
TITLE="Upstream client contract drift: rustdesk $UPSTREAM_BRANCH $SHORT_HEAD"

# Closed issues count too: a closed issue means a human already looked at this sha.
EXISTING=$(gh issue list --label "$ISSUE_LABEL" --state all --limit 500 \
  --json number,title --jq ".[] | select(.title | contains(\"$SHORT_HEAD\")) | .number")
if [ -n "$EXISTING" ]; then
  echo "contract-watch: upstream $SHORT_HEAD already tracked by issue #$(echo "$EXISTING" | head -n1), not opening another" >&2
  exit 0
fi

gh label create "$ISSUE_LABEL" --force \
  --description "Upstream rustdesk client changed files the client API contract depends on" \
  --color D93F0B >/dev/null
gh issue create --title "$TITLE" --label "$ISSUE_LABEL" --body-file "$BODY_FILE"
