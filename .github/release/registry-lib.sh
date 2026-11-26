#!/usr/bin/env bash
# Registry and tag helpers for the release workflow (sourced, not run).
# Need docker buildx, git and jq.

REVISION_LABEL="org.opencontainers.image.revision"

# imagetools inspect --format cannot print a bare digest, so the plain output is parsed.
digest_of() {
  local digest
  digest="$(docker buildx imagetools inspect "$1" | sed -n 's/^Digest: *//p' | head -n 1)"
  [[ "$digest" =~ ^sha256:[0-9a-f]{64}$ ]] || {
    echo "registry-lib: no manifest digest for $1" >&2
    return 1
  }
  echo "$digest"
}

# A platform without the label contributes an empty entry, so equality with one sha means
# every platform carries it.
revisions_of() {
  docker buildx imagetools inspect "$1" --format '{{json .Image}}' |
    jq -r --arg key "$REVISION_LABEL" \
      '[.. | objects | select(has("rootfs")) | ((.config.Labels // {})[$key] // "")] | unique | join(" ")'
}

child_digests() {
  docker buildx imagetools inspect "$1" --raw | jq -r '.manifests[].digest' | sort
}

# 0 exists; 1 only for buildx's exact `ERROR: <ref>: not found` (a message merely containing
# "404" is not that); 2 for any other failure, which callers must treat as an error.
ref_exists() {
  local out
  if out="$(docker buildx imagetools inspect "$1" 2>&1)"; then return 0; fi
  if grep -qxF "ERROR: $1: not found" <<<"$out"; then return 1; fi
  echo "registry-lib: cannot tell whether $1 exists: $out" >&2
  return 2
}

# ls-remote prints an annotated tag's peeled `^{}` line last.
peeled_tag_commit() {
  git ls-remote "$1" "refs/tags/$2" "refs/tags/$2^{}" | awk 'END { print $1 }'
}
