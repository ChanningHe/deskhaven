#!/usr/bin/env bash
# Tags HEAD with the workspace version and pushes the tag, which starts a PUBLISHING
# release run (release.yml). Refuses, before anything leaves this machine, what that run
# would refuse or what would release an untested commit:
#   - HEAD is a clean main and equals main on GitHub
#   - deploy/compose/.env.example pins the workspace version (exactly one DESKHAVEN_TAG line)
#   - ci passed for HEAD in a push run on main
#   - the version is vX.Y.Z[-prerelease] as meta.sh requires
#   - neither tag vX.Y.Z (here or on GitHub) nor a release or draft for it exists (a version
#     is never reused)
# If the push fails, the local tag is deleted again, so the script can simply be re-run.
#
# Usage: version-tag.sh   (run inside the repository). Needs gh, authenticated with access to
# drafts. The repository checked on GitHub is the one the tag is pushed to: origin's push URL.
# DESKHAVEN_REPO=owner/name overrides it for the tests only (they push to a local origin).
set -euo pipefail

fail() {
  echo "version-tag: $*" >&2
  exit 1
}

root="$(git rev-parse --show-toplevel)" || fail "not inside a git repository"
cd "$root"
env_file=deploy/compose/.env.example

repo="${DESKHAVEN_REPO:-$(git remote get-url --push origin | sed -E 's#^(git@github\.com:|ssh://git@github\.com/|https://github\.com/)##; s#\.git$##')}"
[[ "$repo" =~ ^[A-Za-z0-9-]+/[A-Za-z0-9._-]+$ ]] || fail "cannot tell the GitHub repository from '$repo' (set DESKHAVEN_REPO=owner/name)"

version="$(sed -nE '/^\[workspace\.package\]/,/^\[/{s/^version = "([^"]+)".*/\1/p}' Cargo.toml)"
[ -n "$version" ] || fail "workspace version not found in Cargo.toml"
semver='^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z]+(\.[0-9A-Za-z]+)*)?$' # meta.sh's, without the v
[[ "$version" =~ $semver ]] || fail "the workspace version '$version' is not X.Y.Z[-prerelease]: release.yml would refuse its tag"
tag="v$version"
head="$(git rev-parse HEAD)"

[ "$(git symbolic-ref --short -q HEAD || true)" = main ] || fail "not on main"
[ -z "$(git status --porcelain)" ] || fail "the working tree is not clean"
[ "$(grep -c '^DESKHAVEN_TAG=' "$env_file")" = 1 ] || fail "$env_file must have exactly one DESKHAVEN_TAG= line"
grep -qxF "DESKHAVEN_TAG=$version" "$env_file" || fail "$env_file does not pin $version (run: just bump $version)"
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then fail "tag $tag exists locally"; fi

main="$(gh api "repos/$repo/commits/main" --jq .sha)" || fail "cannot read main of $repo"
[ "$main" = "$head" ] || fail "HEAD $head is not main on GitHub ($main): push main first"
green="$(gh api --paginate "repos/$repo/actions/workflows/ci.yml/runs?head_sha=$head&per_page=100" \
  --jq '.workflow_runs[] | select(.event == "push" and .head_branch == "main" and .conclusion == "success") | .id')" ||
  fail "cannot read the ci runs of $head"
[ -n "$green" ] || fail "no successful push ci run on main for $head yet: wait for it (or fix it)"
taken="$(gh api "repos/$repo/git/matching-refs/tags/$tag" --jq '.[] | select(.ref == "refs/tags/'"$tag"'") | .ref')" ||
  fail "cannot read the tags of $repo"
[ -z "$taken" ] || fail "tag $tag exists on GitHub: a version is never reused"
releases="$(gh api --paginate "repos/$repo/releases?per_page=100" --jq '.[] | .tag_name')" ||
  fail "cannot read the releases of $repo"
if grep -qxF "$tag" <<<"$releases"; then fail "a release or draft for $tag exists: a version is never reused"; fi

git tag -a "$tag" -m "$tag"
if ! git push origin "refs/tags/$tag"; then
  git tag -d "$tag" >/dev/null
  fail "pushing $tag failed; the local tag was deleted again"
fi
echo "pushed $tag: the release run starts now (gh run list --workflow release.yml)"
