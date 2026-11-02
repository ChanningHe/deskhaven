#!/usr/bin/env bash
# Verify vendored upstream files against an upstream rustdesk-server checkout.
# Usage: verify-upstream.sh <path-to-upstream-checkout>
# Reads ../UPSTREAM; for each listed file compares the recorded sha256 with
#   (a) `git -C <path> show <commit>:<file>` and (b) the vendored copy.
# Checks out nothing and builds nothing.
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <path-to-upstream-checkout>" >&2
  exit 2
fi

upstream_repo=$1
crate_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
manifest=$crate_dir/UPSTREAM

commit=$(sed -n 's/^commit: //p' "$manifest")
vendored_dir=$(sed -n 's/^vendored-dir: //p' "$manifest")
if [[ ! $commit =~ ^[0-9a-f]{40}$ ]]; then
  echo "error: no valid 40-hex 'commit:' in $manifest" >&2
  exit 2
fi
if [[ -z $vendored_dir ]]; then
  echo "error: no 'vendored-dir:' in $manifest" >&2
  exit 2
fi

git -C "$upstream_repo" cat-file -e "$commit^{commit}"
echo "upstream commit: $(git -C "$upstream_repo" show -s --format='%H %cI' "$commit")"

fail=0
count=0
in_files=0
while read -r sum upath vname; do
  if [[ $in_files -eq 0 ]]; then
    [[ $sum == "files:" ]] && in_files=1
    continue
  fi
  [[ -z ${sum:-} || $sum == \#* ]] && continue
  count=$((count + 1))
  upstream_sum=$(git -C "$upstream_repo" show "$commit:$upath" | sha256sum | cut -d' ' -f1)
  vendored_sum=$(sha256sum "$crate_dir/$vendored_dir/$vname" | cut -d' ' -f1)
  if [[ $upstream_sum == "$sum" && $vendored_sum == "$sum" ]]; then
    echo "OK    $upath  $sum"
  else
    echo "FAIL  $upath  recorded=$sum upstream=$upstream_sum vendored=$vendored_sum" >&2
    fail=1
  fi
done < "$manifest"

if [[ $count -eq 0 ]]; then
  echo "error: no files listed in $manifest" >&2
  exit 2
fi
exit "$fail"
