#!/usr/bin/env bash
# shellcheck source-path=SCRIPTDIR
# Converges `X.Y` and `latest` on the releases published at run time, not on this run's
# version, so a re-run is idempotent and a newer release published meanwhile wins.
#
#   promote.sh decide   prints what this release means for the Latest badge and the tags
#   promote.sh apply    checks, moves (rolling back on failure) and verifies
#
# Environment: VERSION (X.Y.Z[-pre], no v), GITHUB_REPOSITORY, GITHUB_REPOSITORY_OWNER,
# CI_REPO_URL (git remote with the release tags), GH_TOKEN (gh, contents: read). apply also
# needs a docker login with packages: write.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tag="v${VERSION:?VERSION is required}"
images=(deskhaven deskhaven-relay)

# An unreadable list must never read as "no releases".
published_final_tags() {
  gh api --paginate "repos/${GITHUB_REPOSITORY}/releases?per_page=100" \
    --jq '.[] | select(.draft == false and .prerelease == false) | .tag_name'
}

minor_line() { local v="${1#v}"; echo "${v%%-*}" | cut -d. -f1,2; }

desired_moves() {
  local published="$1" highest lines line best
  highest="$("$here/tags.sh" highest <<<"$published")"
  [ -n "$highest" ] || return 0
  lines="$(printf '%s\n%s\n' "$(minor_line "$tag")" "$(minor_line "$highest")" | awk '!seen[$0]++')"
  while IFS= read -r line; do
    best="$("$here/tags.sh" highest "$line" <<<"$published")"
    if [ -n "$best" ]; then echo "${line} ${best#v}"; fi
  done <<<"$lines"
  echo "latest ${highest#v}"
}

decide() {
  local published plan moves
  published="$(published_final_tags)"
  plan="$("$here/tags.sh" plan "$tag" <<<"$published")"
  echo "$plan"
  moves="$(desired_moves "$(printf '%s\n%s\n' "$published" "$tag")")"
  while read -r moving version; do
    if [ -n "$moving" ]; then echo "move_${moving}=v${version}"; fi
  done <<<"$moves"
}

# Uses apply's variables through dynamic scope. A tag that did not exist before cannot be
# deleted through the registry API, so it is acceptable on both images or on neither.
rollback() {
  local attempts="${PROMOTE_RETRY_ATTEMPTS:-3}" pause="${PROMOTE_RETRY_SLEEP:-2}"
  local i entry image moving version name old try ok current
  local -A touched=()
  echo "::error::moving a tag failed; restoring every tag that was attempted"
  for ((i = ${#performed[@]} - 1; i >= 0; i--)); do
    read -r image moving version <<<"${performed[$i]}"
    name="ghcr.io/${owner}/${image}"
    touched["${moving}"]=1
    old="${previous[${image}:${moving}]}"
    [ -n "$old" ] || continue
    ok=0
    for ((try = 1; try <= attempts; try++)); do
      if docker buildx imagetools create --tag "${name}:${moving}" "${name}@${old}"; then
        ok=1
        break
      fi
      echo "::warning::restoring ${name}:${moving} failed (attempt ${try}/${attempts})"
      sleep "$pause"
    done
    [ "$ok" = 1 ] || echo "::error::could not restore ${name}:${moving} to ${old}"
  done
  local -a inconsistent=()
  local -A seen_images=()
  for moving in "${!touched[@]}"; do
    seen_images=()
    for image in "${images[@]}"; do
      name="ghcr.io/${owner}/${image}"
      old="${previous[${image}:${moving}]}"
      current="$(digest_of "${name}:${moving}" 2>/dev/null || true)"
      if [ -n "$old" ]; then
        [ "$current" = "$old" ] || inconsistent+=("${name}:${moving} (expected ${old}, found '${current:-unreadable or absent}')")
      else
        seen_images["$image"]="${current:+present}"
      fi
    done
    if [ -z "${previous[${images[0]}:${moving}]}" ] && [ "${seen_images[${images[0]}]:-}" != "${seen_images[${images[1]}]:-}" ]; then
      inconsistent+=("ghcr.io/${owner}/{${images[0]},${images[1]}}:${moving} exists on only one image and cannot be removed")
    fi
  done
  if [ "${#inconsistent[@]}" -gt 0 ]; then
    printf 'INCONSISTENT: %s\n' "${inconsistent[@]}"
    echo "::error::the images are NOT consistent after the failed promote; fix by hand or re-run promote"
  else
    echo "rolled back: every touched tag is where it was (or on both images)"
  fi
}

apply() {
  # shellcheck source=registry-lib.sh
  . "$here/registry-lib.sh"
  local owner="${GITHUB_REPOSITORY_OWNER,,}" published moves moving version image name
  local -A target_digest=() previous=()
  local -a performed=()

  if [[ "$VERSION" == *-* ]]; then
    echo "prerelease $tag: only its exact tag exists, nothing to promote"
    return 0
  fi
  published="$(published_final_tags)"
  grep -qxF "$tag" <<<"$published" || {
    echo "::error::$tag is not a published release yet: promote runs after the release is published"
    return 1
  }
  moves="$(desired_moves "$published")"
  echo "desired state:"
  while read -r moving version; do
    if [ -n "$moving" ]; then echo "  ${moving} ${version}"; fi
  done <<<"$moves"

  local versions
  versions="$( { echo "$VERSION"; awk '{print $2}' <<<"$moves"; } | sort -u)"
  for version in $versions; do
    local expected
    expected="$(peeled_tag_commit "$CI_REPO_URL" "v${version}")"
    [ -n "$expected" ] || { echo "::error::git tag v${version} does not exist on the remote"; return 1; }
    for image in "${images[@]}"; do
      name="ghcr.io/${owner}/${image}"
      # Tags move from the verified digest, not from the tag, which could change in between.
      [ "$(revisions_of "${name}:${version}")" = "$expected" ] || {
        echo "::error::${name}:${version} does not carry revision ${expected} on every platform (the commit of v${version}); refusing to promote"
        return 1
      }
      target_digest["${image}:${version}"]="$(digest_of "${name}:${version}")"
      [ "$(revisions_of "${name}@${target_digest["${image}:${version}"]}")" = "$expected" ] || {
        echo "::error::${name}@${target_digest["${image}:${version}"]} changed while it was being verified; refusing to promote"
        return 1
      }
    done
  done

  local -a todo=()
  while read -r moving version; do
    [ -n "$moving" ] || continue
    for image in "${images[@]}"; do
      name="ghcr.io/${owner}/${image}"
      if ref_exists "${name}:${moving}"; then
        previous["${image}:${moving}"]="$(digest_of "${name}:${moving}")"
      else
        [ "$?" = 1 ] || return 1
        previous["${image}:${moving}"]=""
      fi
    done
    todo+=("${moving} ${version}")
  done <<<"$moves"

  # Recorded before the create runs: a write the registry applied but reported as failed
  # must still be rolled back.
  local entry failed=0
  for entry in "${todo[@]}"; do
    read -r moving version <<<"$entry"
    for image in "${images[@]}"; do
      name="ghcr.io/${owner}/${image}"
      [ "${previous[${image}:${moving}]}" = "${target_digest[${image}:${version}]}" ] && continue
      performed+=("${image} ${moving} ${version}")
      if ! docker buildx imagetools create --tag "${name}:${moving}" "${name}@${target_digest[${image}:${version}]}"; then
        failed=1
        break 2
      fi
    done
  done
  if [ "$failed" = 1 ]; then
    rollback
    return 1
  fi

  while read -r moving version; do
    [ -n "$moving" ] || continue
    for image in "${images[@]}"; do
      name="ghcr.io/${owner}/${image}"
      [ "$(digest_of "${name}:${moving}")" = "${target_digest[${image}:${version}]}" ] || {
        echo "::error::${name}:${moving} does not point to ${version}"
        return 1
      }
    done
  done <<<"$moves"
  local rev_a rev_b
  rev_a="$(revisions_of "ghcr.io/${owner}/${images[0]}:latest" 2>/dev/null || true)"
  rev_b="$(revisions_of "ghcr.io/${owner}/${images[1]}:latest" 2>/dev/null || true)"
  [ "$rev_a" = "$rev_b" ] || {
    echo "::error::the latest tags of ${images[*]} point to different releases or only one exists: '${rev_a}' vs '${rev_b}'"
    return 1
  }
  echo "promote done"
}

case "${1:-}" in
  decide) decide ;;
  apply) apply ;;
  *)
    echo "usage: promote.sh decide|apply" >&2
    exit 2
    ;;
esac
