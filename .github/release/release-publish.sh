#!/usr/bin/env bash
# shellcheck source-path=SCRIPTDIR
# Creates the GitHub Release as a draft with all assets and publishes it, addressing it by
# id from the moment it exists.
#
# Usage: release-publish.sh ASSETS_DIR BODY_FILE
# Environment: GITHUB_REF_NAME (the tag), GITHUB_SHA, GITHUB_REPOSITORY, CI_REPO_URL,
# VERSION, PRERELEASE, GH_TOKEN (gh, contents: write).
#
#   1. no release for the tag exists, drafts included (the preflight check, repeated here:
#      "re-run failed jobs" replays the old preflight result)
#   2. the tag still points to the commit that was built
#   3. create the draft (`gh api`), keep its id
#   4. upload the assets, check the draft by id: right tag, still a draft, exactly the files
#   5. publish by id (PATCH draft=false and make_latest, which promote.sh decide computes
#      from the releases published right now)
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

releases="$(gh api --paginate "repos/${repo}/releases?per_page=100" --jq '.[] | .tag_name')"
if grep -qxF "$tag" <<<"$releases"; then
  fail "a release or draft for $tag already exists; a release is never replaced (a version is never reused: release the next patch version)"
fi

remote="$(peeled_tag_commit "$CI_REPO_URL" "$tag")"
[ "$remote" = "$GITHUB_SHA" ] || fail "tag $tag points to '$remote', not the built $GITHUB_SHA"

# What the Latest badge should be, from the releases published right now.
latest="$(VERSION="${VERSION:?}" "$here/promote.sh" decide | sed -n 's/^latest=//p')"
[ "$latest" = true ] || latest=false
if [ "${PRERELEASE:-false}" = true ]; then latest=false; fi

id="$(gh api --method POST "repos/${repo}/releases" \
  -f "tag_name=${tag}" -f "name=${tag}" -F "body=@${body}" -F draft=true -F "prerelease=${PRERELEASE:-false}" --jq .id)"
[[ "$id" =~ ^[0-9]+$ ]] || fail "creating the release returned '$id', not an id"
echo "draft release ${id} for ${tag}"

for file in "$assets"/*; do
  gh release upload "$tag" "$file" --repo "$repo"
done

state="$(gh api "repos/${repo}/releases/${id}" --jq '[.tag_name, (.draft | tostring), ([.assets[].name] | sort | join(" "))] | join("|")')"
want="${tag}|true|$(cd "$assets" && find . -maxdepth 1 -type f -printf '%f\n' | LC_ALL=C sort | paste -sd' ' -)"
[ "$state" = "$want" ] || fail "draft ${id} is '${state}', expected '${want}'"

gh api --method PATCH "repos/${repo}/releases/${id}" -F draft=false -f "make_latest=${latest}" >/dev/null
published="$(gh api "repos/${repo}/releases/${id}" --jq '.draft | tostring')"
[ "$published" = false ] || fail "release ${id} is still a draft after publishing"
echo "published release ${id} (${tag}), make_latest=${latest}"
