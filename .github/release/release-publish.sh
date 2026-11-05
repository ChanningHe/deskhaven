#!/usr/bin/env bash
# shellcheck source-path=SCRIPTDIR
# Creates the GitHub Release as a draft with all assets and publishes it, addressing it by
# id from the moment it exists.
#
# Usage: release-publish.sh ASSETS_DIR BODY_FILE
# Environment: GITHUB_REF_NAME (the tag), GITHUB_SHA, GITHUB_REPOSITORY, CI_REPO_URL,
# VERSION, PRERELEASE, GH_TOKEN (gh, contents: write). ASSETS_DIR holds the assets and their
# SHA256SUMS.
#
#   1. a release for the tag exists?
#        no   -> continue
#        yes  -> only ONE case is fine: it is published (not a draft), the tag points to the
#                commit that was built, and its assets are exactly these files with GitHub's
#                digest of each equal to SHA256SUMS. That is this very release, published by
#                an earlier attempt that failed afterwards: succeed without touching it, so
#                the jobs after it can run. Anything else (a draft, other content) fails.
#   2. the tag points to the commit that was built
#   3. create the draft (`gh api`), keep its id
#   4. upload every asset to that id through the uploads API
#   5. check the draft by id: right tag, still a draft, exactly the files, and every asset's
#      GitHub digest equal to the sha256 recorded in SHA256SUMS
#   6. the tag still points to the built commit (checked again right before publishing)
#   7. publish by id (PATCH draft=false and make_latest, which promote.sh decide computes
#      from the releases published right now), then check it is no longer a draft
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=registry-lib.sh
. "$here/registry-lib.sh"

assets="${1:?usage: release-publish.sh ASSETS_DIR BODY_FILE}"
body="${2:?usage: release-publish.sh ASSETS_DIR BODY_FILE}"
tag="${GITHUB_REF_NAME:?}"
repo="${GITHUB_REPOSITORY:?}"

fail() {
  echo "::error::$*"
  exit 1
}

tag_is_built_commit() {
  [ "$(peeled_tag_commit "$CI_REPO_URL" "$tag")" = "$GITHUB_SHA" ]
}

# want_assets: "name=sha256:hash" for every file, sorted, from SHA256SUMS (and the hash of
# SHA256SUMS itself); every file in the directory must have an entry.
want_assets() {
  local sums="${assets}/SHA256SUMS" lines="" file name hash
  [ -f "$sums" ] && [ ! -L "$sums" ] || fail "no SHA256SUMS in ${assets}"
  (cd "$assets" && sha256sum -c SHA256SUMS --quiet) >&2 || fail "the assets do not match SHA256SUMS"
  while read -r hash name; do
    lines+="${name#\*}=sha256:${hash}"$'\n'
  done <"$sums"
  lines+="SHA256SUMS=sha256:$(sha256sum "$sums" | cut -d' ' -f1)"$'\n'
  for file in "$assets"/*; do
    name="$(basename "$file")"
    grep -q "^${name}=" <<<"$lines" || fail "${name} is not listed in SHA256SUMS"
  done
  printf '%s' "$lines" | LC_ALL=C sort | paste -sd' ' -
}

# release_state ID: "tag|draft|name=digest name=digest ..." as GitHub reports it
release_state() {
  gh api "repos/${repo}/releases/${1}" \
    --jq '[.tag_name, (.draft | tostring), ([.assets[] | .name + "=" + (.digest // "none")] | sort | join(" "))] | join("|")'
}

want="$(want_assets)"

existing="$(gh api --paginate "repos/${repo}/releases?per_page=100" --jq '.[] | [.tag_name, (.id | tostring)] | @tsv')"
existing_id="$(awk -F'\t' -v t="$tag" '$1 == t { print $2 }' <<<"$existing" | head -n 1)"
if [ -n "$existing_id" ]; then
  tag_is_built_commit || fail "a release for $tag exists and the tag no longer points to the built commit $GITHUB_SHA"
  state="$(release_state "$existing_id")"
  if [ "$state" = "${tag}|false|${want}" ]; then
    echo "release ${existing_id} for ${tag} is already published with exactly these assets (an earlier attempt of this run); nothing to do"
    exit 0
  fi
  fail "a release or draft for $tag already exists (${existing_id}: '${state}') and is not exactly this run's published release; a release is never replaced (a version is never reused: release the next patch version)"
fi

tag_is_built_commit || fail "tag $tag points to '$(peeled_tag_commit "$CI_REPO_URL" "$tag")', not the built $GITHUB_SHA"

# What the Latest badge should be, from the releases published right now.
latest="$(VERSION="${VERSION:?}" "$here/promote.sh" decide | sed -n 's/^latest=//p')"
[ "$latest" = true ] || latest=false
if [ "${PRERELEASE:-false}" = true ]; then latest=false; fi

id="$(gh api --method POST "repos/${repo}/releases" \
  -f "tag_name=${tag}" -f "name=${tag}" -F "body=@${body}" -F draft=true -F "prerelease=${PRERELEASE:-false}" --jq .id)"
[[ "$id" =~ ^[0-9]+$ ]] || fail "creating the release returned '$id', not an id"
echo "draft release ${id} for ${tag}"

# Assets go to the release BY ID (the uploads API), never found by tag.
for file in "$assets"/*; do
  name="$(basename "$file")"
  gh api --method POST "https://uploads.github.com/repos/${repo}/releases/${id}/assets?name=${name}" \
    -H "Content-Type: application/octet-stream" --input "$file" >/dev/null
done

state="$(release_state "$id")"
[ "$state" = "${tag}|true|${want}" ] || fail "draft ${id} is '${state}', expected '${tag}|true|${want}' (assets and their digests)"

tag_is_built_commit || fail "tag $tag moved while the release was being prepared; not publishing"

gh api --method PATCH "repos/${repo}/releases/${id}" -F draft=false -f "make_latest=${latest}" >/dev/null
published="$(gh api "repos/${repo}/releases/${id}" --jq '.draft | tostring')"
[ "$published" = false ] || fail "release ${id} is still a draft after publishing"
echo "published release ${id} (${tag}), make_latest=${latest}"
