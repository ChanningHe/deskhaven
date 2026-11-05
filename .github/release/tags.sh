#!/usr/bin/env bash
# Semver ordering of the repository's `v*` tags, for the release workflow.
#
#   tags.sh plan TAG   prints `latest=true|false` and `minor_latest=true|false`:
#                        latest        TAG is a final release and the highest final release
#                        minor_latest  TAG is a final release and the highest in its X.Y line
#                      (TAG itself need not exist yet: a dry run asks about the version
#                      it would release.)
#   tags.sh prev TAG   prints the tag whose release notes end where TAG's begin, or nothing:
#                        final TAG       the highest FINAL tag below it (prereleases are
#                                        skipped, so the notes of v1.0.0 also cover the rcs)
#                        prerelease TAG  the highest tag of any kind below it
#                      "Below" is semver order, not creation order, so a patch tag for an
#                      old line (v0.1.1 after v0.2.0) gets v0.1.0, not v0.2.0.
#
# Ordering: major.minor.patch numerically; a prerelease sorts before its final release;
# prerelease identifiers compare per semver (numeric ones numerically, and below
# alphanumeric ones). Tags that are not vX.Y.Z[-prerelease] are ignored.
set -euo pipefail
export LC_ALL=C

SEMVER='^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z]+(\.[0-9A-Za-z]+)*)?$'
FINAL='^v[0-9]+\.[0-9]+\.[0-9]+$'

# Sort key of a tag: byte-wise (LC_ALL=C) order of the keys is semver order.
key() {
  awk -v tag="$1" 'BEGIN {
    s = substr(tag, 2)
    dash = index(s, "-")
    core = s; pre = ""
    if (dash > 0) { core = substr(s, 1, dash - 1); pre = substr(s, dash + 1) }
    split(core, n, ".")
    out = sprintf("%010d.%010d.%010d", n[1], n[2], n[3])
    if (pre == "") { print out " 1"; exit }
    count = split(pre, ids, ".")
    q = ""
    for (i = 1; i <= count; i++) {
      if (ids[i] ~ /^[0-9]+$/) q = q sprintf("0%010d.", ids[i])
      else q = q "1" ids[i] "."
    }
    print out " 0 " q
  }'
}

# All valid tags (plus the extra one given), one per line, "key<TAB>tag", sorted ascending.
sorted_tags() {
  local extra="$1" tag
  {
    git tag -l 'v*'
    [ -z "$extra" ] || echo "$extra"
  } | sort -u | while IFS= read -r tag; do
    if [[ "$tag" =~ $SEMVER ]]; then
      printf '%s\t%s\n' "$(key "$tag")" "$tag"
    fi
  done | LC_ALL=C sort
}

minor_line() { local v="${1#v}"; echo "${v%%-*}" | cut -d. -f1,2; }

case "${1:-}" in
  plan)
    tag="${2:?usage: tags.sh plan TAG}"
    [[ "$tag" =~ $SEMVER ]] || { echo "tags.sh: '$tag' is not vX.Y.Z[-prerelease]" >&2; exit 2; }
    latest=false
    minor_latest=false
    if [[ "$tag" =~ $FINAL ]]; then
      finals="$(sorted_tags "$tag" | cut -f2 | grep -E "$FINAL")"
      if [ "$(tail -n 1 <<<"$finals")" = "$tag" ]; then latest=true; fi
      line="$(minor_line "$tag")"
      in_line=""
      while IFS= read -r t; do
        if [ "$(minor_line "$t")" = "$line" ]; then in_line="$t"; fi
      done <<<"$finals"
      if [ "$in_line" = "$tag" ]; then minor_latest=true; fi
    fi
    echo "latest=$latest"
    echo "minor_latest=$minor_latest"
    ;;
  prev)
    tag="${2:?usage: tags.sh prev TAG}"
    [[ "$tag" =~ $SEMVER ]] || { echo "tags.sh: '$tag' is not vX.Y.Z[-prerelease]" >&2; exit 2; }
    own_key="$(key "$tag")"
    candidates="$(sorted_tags "")"
    prev=""
    while IFS=$'\t' read -r k t; do
      [ -n "$t" ] || continue
      [ "$t" != "$tag" ] || continue
      [[ "$k" < "$own_key" ]] || continue
      if [[ "$tag" =~ $FINAL ]] && ! [[ "$t" =~ $FINAL ]]; then continue; fi
      prev="$t"
    done <<<"$candidates"
    echo "$prev"
    ;;
  *)
    echo "usage: tags.sh plan TAG | prev TAG" >&2
    exit 2
    ;;
esac
