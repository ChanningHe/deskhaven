# Release chores. A release is two steps: `just bump X.Y.Z` commits the version, you push
# main and wait for its ci run, then `just tag` pushes vX.Y.Z, which publishes it
# (.github/workflows/release.yml). A version is never reused.

_default:
    @just --list

# Set the version everywhere it is pinned and commit it on main (nothing is pushed)
bump version:
    .github/release/version-bump.sh {{ quote(version) }}

# Tag HEAD with the workspace version and push the tag: this PUBLISHES the release
[confirm("Push the release tag of the workspace version? This publishes the release. [y/N]")]
tag:
    .github/release/version-tag.sh
