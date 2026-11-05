#!/usr/bin/env bash
# shellcheck source-path=SCRIPTDIR
# Moves the image tags `X.Y` and `latest` after a release is published, deciding AT RUN TIME
# from the published GitHub releases (non-draft, non-prerelease), never from git tags or
# from what an earlier job computed: re-running this job alone later is safe, and a newer
# release published meanwhile is respected.
#
#   promote.sh decide        prints latest=true|false and minor_latest=true|false for this
#                            release (tags.sh plan over the published releases)
#   promote.sh apply         moves the tags of BOTH images as one step, then verifies them
#
# Environment: VERSION (X.Y.Z[-pre], no v), GITHUB_REPOSITORY, GITHUB_REPOSITORY_OWNER,
# GH_TOKEN (gh, contents: read); apply also needs a docker login with packages: write.
# Idempotent: the tags are only ever created from `<image>:<VERSION>`.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tag="v${VERSION:?VERSION is required}"

# Tags of the published final releases, one per line. Any API error fails (set -e through
# the assignment): an unreadable list must never read as "no releases".
published_final_tags() {
  gh api --paginate "repos/${GITHUB_REPOSITORY}/releases?per_page=100" \
    --jq '.[] | select(.draft == false and .prerelease == false) | .tag_name'
}

decide() {
  local published
  published="$(published_final_tags)"
  "$here/tags.sh" plan "$tag" <<<"$published"
}

case "${1:-}" in
  decide)
    decide
    ;;
  apply)
    # shellcheck source=registry-lib.sh
    . "$here/registry-lib.sh"
    owner="${GITHUB_REPOSITORY_OWNER,,}"
    images=(deskhaven deskhaven-relay)

    if [[ "$VERSION" == *-* ]]; then
      echo "prerelease $tag: only its exact tag exists, nothing to promote"
      exit 0
    fi
    published="$(published_final_tags)"
    grep -qxF "$tag" <<<"$published" || {
      echo "::error::$tag is not a published release yet: promote runs after the release is published"
      exit 1
    }
    plan="$("$here/tags.sh" plan "$tag" <<<"$published")"
    echo "$plan"
    latest="$(sed -n 's/^latest=//p' <<<"$plan")"
    minor_latest="$(sed -n 's/^minor_latest=//p' <<<"$plan")"

    moving=()
    if [ "$minor_latest" = true ]; then moving+=("${VERSION%.*}"); fi
    if [ "$latest" = true ]; then moving+=(latest); fi
    echo "moving tags: ${moving[*]:-none}"

    # One step for both images: every exact tag is checked before anything moves.
    for image in "${images[@]}"; do
      digest_of "ghcr.io/${owner}/${image}:${VERSION}" >/dev/null
    done
    if [ "${#moving[@]}" -gt 0 ]; then
      for image in "${images[@]}"; do
        name="ghcr.io/${owner}/${image}"
        tag_args=()
        for t in "${moving[@]}"; do tag_args+=(--tag "${name}:${t}"); done
        docker buildx imagetools create "${tag_args[@]}" "${name}:${VERSION}"
      done
    fi

    # Verify: every moved tag is the exact release, and both images' `latest` are one release.
    for image in "${images[@]}"; do
      name="ghcr.io/${owner}/${image}"
      exact="$(digest_of "${name}:${VERSION}")"
      for t in "${moving[@]}"; do
        [ "$(digest_of "${name}:${t}")" = "$exact" ] || {
          echo "::error::${name}:${t} does not point to ${VERSION}"
          exit 1
        }
      done
    done
    revisions=()
    for image in "${images[@]}"; do
      revisions+=("$(revision_of "ghcr.io/${owner}/${image}:latest" 2>/dev/null || true)")
    done
    # Both `latest` tags exist and carry the same revision, or neither exists.
    if [ "${revisions[0]}" != "${revisions[1]}" ]; then
      echo "::error::the latest tags of ${images[*]} point to different releases or only one exists: '${revisions[0]}' vs '${revisions[1]}'"
      exit 1
    fi
    echo "promote done"
    ;;
  *)
    echo "usage: promote.sh decide|apply" >&2
    exit 2
    ;;
esac
