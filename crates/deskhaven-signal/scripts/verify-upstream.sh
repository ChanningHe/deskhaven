#!/usr/bin/env bash
# Verify the vendored upstream hbbs files against an upstream rustdesk-server checkout.
#
# Usage: verify-upstream.sh [--worktree] <path-to-upstream-checkout>
#
# Reads ../UPSTREAM. For every file listed there it compares, byte for byte (via sha256):
#   (a) `git show <commit>:<upstream-path>`          in the upstream checkout,
#   (b) `git show <vendored-commit>:<vendored-path>`  in this repository's history,
#   (c) the sha256 recorded in the manifest.
# It also checks the recorded upstream commit date, and that the vendoring commit contains
# no unlisted files under <vendored-dir>. Reading history (not the working tree) keeps the
# check valid after the files are later modified or moved.
#
# --worktree additionally requires the CURRENT working tree's <vendored-dir> to be pristine
# (listed files match, no extra files). Only meaningful until the first modification.
#
# Checks out nothing, builds nothing. Never aborts on a single failure: prints
# `FAIL <file> <reason>` for each problem and exits 1 at the end if any occurred.
set -euo pipefail

check_worktree=0
if [[ ${1:-} == "--worktree" ]]; then
  check_worktree=1
  shift
fi
if [[ $# -ne 1 ]]; then
  echo "usage: $0 [--worktree] <path-to-upstream-checkout>" >&2
  exit 2
fi

upstream_repo=$1
script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
crate_dir=$(dirname "$script_dir")
manifest=$crate_dir/UPSTREAM
failures=0

fail() { # fail <subject> <reason>
  echo "FAIL  $1  $2" >&2
  failures=$((failures + 1))
}

manifest_value() { # manifest_value <key>
  sed -n "s/^$1: //p" "$manifest" | head -n 1
}

# blob_sha <repo> <rev> <path>: print sha256 of the blob, return non-zero if it cannot be read.
blob_sha() {
  local out
  out=$(git -C "$1" show "$2:$3" | sha256sum) || return 1
  printf '%s\n' "${out%% *}"
}

if [[ ! -f $manifest ]]; then
  echo "error: manifest not found: $manifest" >&2
  exit 2
fi

commit=$(manifest_value commit)
recorded_date=$(manifest_value date)
vendored_commit=$(manifest_value vendored-commit)
vendored_dir=$(manifest_value vendored-dir)
for pair in "commit:$commit" "vendored-commit:$vendored_commit"; do
  if [[ ! ${pair#*:} =~ ^[0-9a-f]{40}$ ]]; then
    echo "error: no valid 40-hex '${pair%%:*}:' in $manifest" >&2
    exit 2
  fi
done
if [[ -z $vendored_dir || -z $recorded_date ]]; then
  echo "error: 'vendored-dir:' and 'date:' are required in $manifest" >&2
  exit 2
fi

if ! repo_root=$(git -C "$script_dir" rev-parse --show-toplevel); then
  echo "error: $script_dir is not inside a git repository" >&2
  exit 2
fi
vendored_prefix=${crate_dir#"$repo_root"/}/$vendored_dir

if ! git -C "$upstream_repo" cat-file -e "$commit^{commit}"; then
  echo "error: upstream commit $commit not found in $upstream_repo" >&2
  exit 2
fi
if ! git -C "$repo_root" cat-file -e "$vendored_commit^{commit}"; then
  echo "error: vendored-commit $vendored_commit not found in this repository" >&2
  exit 2
fi

actual_date=$(git -C "$upstream_repo" show -s --format=%cI "$commit")
echo "upstream commit:  $commit"
echo "upstream date:    recorded=$recorded_date actual=$actual_date"
echo "vendored commit:  $vendored_commit"
if [[ $actual_date != "$recorded_date" ]]; then
  fail "UPSTREAM" "date mismatch: recorded=$recorded_date actual=$actual_date"
fi

# Parse the file list. `|| [[ -n ... ]]` handles a final line without trailing newline.
listed=()
in_files=0
while read -r sum upath vpath || [[ -n ${sum:-} ]]; do
  if [[ $in_files -eq 0 ]]; then
    [[ ${sum:-} == "files:" ]] && in_files=1
    continue
  fi
  [[ -z ${sum:-} || $sum == \#* ]] && continue
  if [[ ! $sum =~ ^[0-9a-f]{64}$ || -z ${upath:-} || -z ${vpath:-} ]]; then
    fail "UPSTREAM" "malformed file entry: $sum ${upath:-} ${vpath:-}"
    continue
  fi
  listed+=("$vpath")

  if ! up_sum=$(blob_sha "$upstream_repo" "$commit" "$upath"); then
    fail "$upath" "cannot read from upstream at $commit"
    continue
  fi
  if ! v_sum=$(blob_sha "$repo_root" "$vendored_commit" "$vpath"); then
    fail "$vpath" "cannot read from vendored commit $vendored_commit"
    continue
  fi
  if [[ $up_sum != "$sum" ]]; then
    fail "$upath" "upstream sha256 $up_sum != recorded $sum"
  elif [[ $v_sum != "$up_sum" ]]; then
    fail "$vpath" "vendored sha256 $v_sum != upstream $up_sum"
  else
    echo "OK    $upath  $sum"
  fi

  if [[ $check_worktree -eq 1 ]]; then
    if [[ ! -f $repo_root/$vpath ]]; then
      fail "$vpath" "missing from working tree"
    elif [[ $(sha256sum "$repo_root/$vpath" | cut -d' ' -f1) != "$sum" ]]; then
      fail "$vpath" "working tree copy differs from recorded sha256"
    fi
  fi
done < "$manifest"

if [[ ${#listed[@]} -eq 0 ]]; then
  fail "UPSTREAM" "no files listed"
fi

# Unlisted files: the vendoring commit must contain exactly the listed files under the dir.
is_listed() { # is_listed <path>
  local p
  for p in "${listed[@]}"; do [[ $p == "$1" ]] && return 0; done
  return 1
}
if ! committed=$(git -C "$repo_root" ls-tree -r --name-only "$vendored_commit" -- "$vendored_prefix"); then
  fail "$vendored_prefix" "cannot list tree at $vendored_commit"
  committed=""
fi
while IFS= read -r path; do
  [[ -z $path ]] && continue
  is_listed "$path" || fail "$path" "unlisted file in vendoring commit"
done <<< "$committed"

if [[ $check_worktree -eq 1 && -d $repo_root/$vendored_prefix ]]; then
  while IFS= read -r path; do
    is_listed "$path" || fail "$path" "unlisted file in working tree (--worktree)"
  done < <(cd "$repo_root" && find "$vendored_prefix" -type f | sort)
fi

if [[ $failures -ne 0 ]]; then
  echo "verify-upstream: $failures failure(s)" >&2
  exit 1
fi
echo "verify-upstream: all checks passed"
