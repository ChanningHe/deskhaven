#!/usr/bin/env bash
# Registry and tag helpers for the release workflow (sourced, not run).
# Need docker buildx, git and jq.

REVISION_LABEL="org.opencontainers.image.revision"

# digest_of REF: the digest of the manifest or index REF points to.
# (`--format` of imagetools inspect does not print a bare digest, so the plain output is parsed.)
digest_of() {
  local digest
  digest="$(docker buildx imagetools inspect "$1" | sed -n 's/^Digest: *//p' | head -n 1)"
  [[ "$digest" =~ ^sha256:[0-9a-f]{64}$ ]] || {
    echo "registry-lib: no manifest digest for $1" >&2
    return 1
  }
  echo "$digest"
}

# revisions_of REF: the org.opencontainers.image.revision labels of EVERY platform image of
# REF, sorted and de-duplicated, joined by spaces; a platform without the label contributes
# an empty entry. `[ "$(revisions_of REF)" = "$sha" ]` therefore means: all platforms carry
# exactly that revision. (Image configs are the objects with a `rootfs`, one per platform.)
revisions_of() {
  docker buildx imagetools inspect "$1" --format '{{json .Image}}' |
    jq -r --arg key "$REVISION_LABEL" \
      '[.. | objects | select(has("rootfs")) | ((.config.Labels // {})[$key] // "")] | unique | join(" ")'
}

# child_digests REF: the digests of the manifests an index lists, sorted, one per line.
child_digests() {
  docker buildx imagetools inspect "$1" --raw | jq -r '.manifests[].digest' | sort
}

# ref_exists REF: 0 if the tag or digest exists, 1 if the registry says it does not exist,
# in buildx's exact words `ERROR: <ref>: not found` (verified against registry:2; a message
# that merely CONTAINS "404" or "not found", such as a connection reset to port 40404, is
# not that); any other failure returns 2 and must be treated as an error.
ref_exists() {
  local out
  if out="$(docker buildx imagetools inspect "$1" 2>&1)"; then return 0; fi
  if grep -qxF "ERROR: $1: not found" <<<"$out"; then return 1; fi
  echo "registry-lib: cannot tell whether $1 exists: $out" >&2
  return 2
}

# peeled_tag_commit URL TAG: the commit TAG points to on the remote (annotated tags are
# peeled: ls-remote prints the `^{}` line last), or empty if the tag does not exist.
peeled_tag_commit() {
  git ls-remote "$1" "refs/tags/$2" "refs/tags/$2^{}" | awk 'END { print $1 }'
}
