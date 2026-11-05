#!/usr/bin/env bash
# Registry helpers for the release workflow (sourced, not run). Need docker buildx and jq.

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

# revision_of REF: the org.opencontainers.image.revision label of the image (of the first
# platform if REF is an index), or empty.
revision_of() {
  docker buildx imagetools inspect "$1" --format '{{json .Image}}' |
    jq -r --arg key "$REVISION_LABEL" '[.. | objects | .Labels? // empty | .[$key]?] | map(select(. != null)) | first // empty'
}
