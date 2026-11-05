#!/usr/bin/env bash
# shellcheck source-path=SCRIPTDIR
# Creates the exact tag X.Y.Z (a multi-arch index of the per-arch images pushed by digest)
# for BOTH images, and writes images.txt. The moving tags (X.Y, latest) are promote.sh's.
#
# "Re-run failed jobs" replays old outputs, so nothing from the run is trusted blindly.
# Before ANY index is created:
#   - the git tag still points to the commit this run built;
#   - every per-arch digest is well-formed and its image carries that commit as revision;
#   - an image that already has the tag X.Y.Z is accepted only if its index lists EXACTLY
#     this run's per-arch digests (a genuine re-run of this very run); anything else fails.
# Both images are checked before the first index is created, so the window in which one
# image has the tag and the other has not is as small as a failure of the second create.
#
# Environment: VERSION, GITHUB_SHA, GITHUB_REF_NAME, CI_REPO_URL, GITHUB_REPOSITORY_OWNER;
# arguments: DIGESTS_DIR (files <deskhaven|relay>-<amd64|arm64>.txt) OUTPUT_FILE.
# Needs a docker login with packages: write.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=registry-lib.sh
. "$here/registry-lib.sh"

digests_dir="${1:?usage: exact-tags.sh DIGESTS_DIR OUTPUT_FILE}"
output="${2:?usage: exact-tags.sh DIGESTS_DIR OUTPUT_FILE}"
owner="${GITHUB_REPOSITORY_OWNER,,}"

fail() {
  echo "::error::$*"
  exit 1
}

[ "$(peeled_tag_commit "$CI_REPO_URL" "$GITHUB_REF_NAME")" = "$GITHUB_SHA" ] ||
  fail "tag $GITHUB_REF_NAME no longer points to the built commit $GITHUB_SHA"

# matrix name (as in the digest files) -> repository name
pairs=(deskhaven:deskhaven relay:deskhaven-relay)
declare -A sources=() create=() expected=()

for pair in "${pairs[@]}"; do
  key="${pair%%:*}"
  name="ghcr.io/${owner}/${pair#*:}"
  list=()
  for arch in amd64 arm64; do
    file="${digests_dir}/${key}-${arch}.txt"
    [ -f "$file" ] && [ ! -L "$file" ] || fail "missing digest file ${file}"
    digest="$(cat "$file")"
    [[ "$digest" =~ ^sha256:[0-9a-f]{64}$ ]] || fail "invalid digest for ${key} ${arch}: '$digest'"
    [ "$(revisions_of "${name}@${digest}")" = "$GITHUB_SHA" ] ||
      fail "${name}@${digest} does not carry revision $GITHUB_SHA"
    list+=("${name}@${digest}")
  done
  sources[$pair]="${list[*]}"
  expected[$pair]="$(printf '%s\n' "${list[@]}" | sed 's/.*@//' | sort)"

  create[$pair]=yes
  if ref_exists "${name}:${VERSION}"; then
    [ "$(child_digests "${name}:${VERSION}")" = "${expected[$pair]}" ] ||
      fail "${name}:${VERSION} already exists with different content: a version is never reused, release the next patch version"
    create[$pair]=no
    echo "${name}:${VERSION} already exists with exactly this run's images (a re-run)"
  else
    [ "$?" = 1 ] || exit 1
  fi
done

: >"$output"
for pair in "${pairs[@]}"; do
  name="ghcr.io/${owner}/${pair#*:}"
  if [ "${create[$pair]}" = yes ]; then
    read -r -a list <<<"${sources[$pair]}"
    docker buildx imagetools create --tag "${name}:${VERSION}" "${list[@]}"
  fi
  # Whatever the create said, the tag must now list exactly this run's images: a create
  # that "succeeded" onto something else, or raced with another writer, must not be recorded.
  [ "$(child_digests "${name}:${VERSION}")" = "${expected[$pair]}" ] ||
    fail "${name}:${VERSION} does not list this run's images after the create"
  echo "${name}@$(digest_of "${name}:${VERSION}")  (tag ${VERSION})" >>"$output"
done
cat "$output"
