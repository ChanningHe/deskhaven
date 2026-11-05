#!/usr/bin/env bash
# Semver ordering of release tags, for the release workflow.
#
#   tags.sh plan TAG    reads the tags of the PUBLISHED final releases on stdin, one per
#                       line (from the GitHub releases API, never from raw git tags: an
#                       unreleased stray tag must not freeze `latest`), and prints
#                       `latest=true|false` and `minor_latest=true|false`:
#                         latest        TAG is final and the highest published final release
#                         minor_latest  TAG is final and the highest in its X.Y line
#                       TAG counts as published whether or not it is in the list yet.
#   tags.sh highest [X.Y]
#                       reads published final release tags on stdin and prints the highest
#                       one (of the X.Y line if given), or nothing
#   tags.sh prev TAG    reads the tags of ALL published releases (drafts excluded) on stdin
#                       and prints the tag whose release notes end where TAG's begin, or
#                       nothing:
#                         final TAG       the highest FINAL tag below it (prereleases are
#                                         skipped, so the notes of v1.0.0 also cover the rcs)
#                         prerelease TAG  the highest tag of any kind below it
#                       "Below" is semver order, not creation order, so a patch tag for an
#                       old line (v0.1.1 after v0.2.0) gets v0.1.0, not v0.2.0.
#
# Ordering: major.minor.patch numerically (any number of digits); a prerelease sorts before
# its final release; prerelease identifiers compare per semver (numeric ones numerically,
# and below alphanumeric ones). Tags that are not vX.Y.Z[-prerelease] are ignored.
set -euo pipefail
export LC_ALL=C

SEMVER='^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z]+(\.[0-9A-Za-z]+)*)?$'
FINAL='^v[0-9]+\.[0-9]+\.[0-9]+$'

# Sort key of a tag: byte-wise (LC_ALL=C) order of the keys is semver order. A number is
# written as its digit count (3 digits) followed by its digits, so longer numbers sort
# after shorter ones whatever their size.
key() {
  awk -v tag="$1" '
    function num(x) { sub(/^0+/, "", x); if (x == "") x = "0"; return sprintf("%03d%s", length(x), x) }
    BEGIN {
      s = substr(tag, 2)
      dash = index(s, "-")
      core = s; pre = ""
      if (dash > 0) { core = substr(s, 1, dash - 1); pre = substr(s, dash + 1) }
      split(core, n, ".")
      out = num(n[1]) "." num(n[2]) "." num(n[3])
      if (pre == "") { print out " 1"; exit }
      count = split(pre, ids, ".")
      q = ""
      for (i = 1; i <= count; i++) {
        if (ids[i] ~ /^[0-9]+$/) q = q "0" num(ids[i]) "."
        else q = q "1" ids[i] "."
      }
      print out " 0 " q
    }'
}

# Tags from stdin (plus the extra one given), valid ones only, as "key<TAB>tag" sorted ascending.
sorted() {
  local extra="$1" tag
  {
    cat
    [ -z "$extra" ] || echo "$extra"
  } | sort -u | while IFS= read -r tag; do
    if [[ "$tag" =~ $SEMVER ]]; then
      printf '%s\t%s\n' "$(key "$tag")" "$tag"
    fi
  done | sort
}

minor_line() { local v="${1#v}"; echo "${v%%-*}" | cut -d. -f1,2; }

case "${1:-}" in
  plan)
    tag="${2:?usage: tags.sh plan TAG (published final release tags on stdin)}"
    [[ "$tag" =~ $SEMVER ]] || { echo "tags.sh: '$tag' is not vX.Y.Z[-prerelease]" >&2; exit 2; }
    latest=false
    minor_latest=false
    if [[ "$tag" =~ $FINAL ]]; then
      finals="$(sorted "$tag" | cut -f2 | grep -E "$FINAL")"
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
    prev=""
    while IFS=$'\t' read -r k t; do
      [ -n "$t" ] || continue
      [ "$t" != "$tag" ] || continue
      [[ "$k" < "$own_key" ]] || continue
      if [[ "$tag" =~ $FINAL ]] && ! [[ "$t" =~ $FINAL ]]; then continue; fi
      prev="$t"
    done < <(sorted "")
    echo "$prev"
    ;;
  highest)
    line="${2:-}"
    best=""
    while IFS=$'\t' read -r _ t; do
      [ -n "$t" ] || continue
      [[ "$t" =~ $FINAL ]] || continue
      if [ -n "$line" ] && [ "$(minor_line "$t")" != "$line" ]; then continue; fi
      best="$t"
    done < <(sorted "")
    echo "$best"
    ;;
  *)
    echo "usage: tags.sh plan TAG | highest [X.Y] | prev TAG  (tags on stdin)" >&2
    exit 2
    ;;
esac
