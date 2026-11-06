#!/usr/bin/env bash
# version-bump.sh: what a bump rewrites (and leaves alone), and every refusal, in a scratch
# repository with a stubbed cargo.
set -euo pipefail
# shellcheck source=lib.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

# cargo stub: logs its arguments; STUB_CARGO_FAIL=1 makes it fail
mkdir -p "$TMP/bin"
cat >"$TMP/bin/cargo" <<'EOF'
#!/usr/bin/env bash
echo "cargo $*" >>"$CARGO_LOG"
[ "${STUB_CARGO_FAIL:-0}" != 1 ] || exit 1
if [ "$*" = "xtask gen-openapi" ]; then echo "generated" >openapi.json; fi
EOF
chmod +x "$TMP/bin/cargo"
export PATH="$TMP/bin:$PATH"
export CARGO_LOG="$TMP/cargo.log"

repo="$TMP/repo"
mkdir -p "$repo/deploy/compose" "$repo/docs/guides"
cd "$repo"
git init -q -b main .
git config user.email t@example.com
git config user.name t
cat >Cargo.toml <<'EOF'
[workspace.package]
version = "0.1.0-dev"
edition = "2024"

[workspace.dependencies]
serde = { version = "1.0" }

[other]
version = "9.9.9"
EOF
printf '# header\nDESKHAVEN_TAG=0.1.0\nOTHER=1\n' >deploy/compose/.env.example
cat >docs/guides/install.md <<'EOF'
[v0.1.0 release](https://github.com/ChanningHe/deskhaven/releases/tag/v0.1.0), `hbbr` from
the [rustdesk-server 1.1.16 release](https://github.com/rustdesk/rustdesk-server/releases/tag/1.1.16)
base=https://raw.githubusercontent.com/ChanningHe/deskhaven/v0.1.0/deploy/compose
Needs Docker Compose v2.20.0 or later.
Get [v0.1.0 release](https://github.com/ChanningHe/deskhaven/releases/download/v0.1.0/x.tar.gz) (Docker v2.20.0+)
See https://github.com/ChanningHe/deskhaven/blob/v0.1.0/README.md
EOF
printf 'See https://example.com/mirror/v0.1.0/x\n' >docs/other.md
echo lock >Cargo.lock
echo spec >openapi.json
git add -A
git commit -q -m "feat: base"

bump() { bash "$RELEASE_DIR/version-bump.sh" "$@"; }
commits() { git rev-list --count HEAD; }
workspace_version() { sed -nE '/^\[workspace\.package\]/,/^\[/{s/^version = "([^"]+)".*/\1/p}' Cargo.toml; }

echo "== a pre-release bump"
: >"$CARGO_LOG"
docs_before="$(cat docs/guides/install.md)"
expect_pass "bump to 0.1.0-rc.1" bump 0.1.0-rc.1
assert_eq "workspace version" "0.1.0-rc.1" "$(workspace_version)"
assert_contains "another table's version is untouched" 'version = "9.9.9"' "$(cat Cargo.toml)"
assert_contains "a dependency's version is untouched" 'serde = { version = "1.0" }' "$(cat Cargo.toml)"
assert_eq "compose pins the version" "# header|DESKHAVEN_TAG=0.1.0-rc.1|OTHER=1" "$(paste -sd'|' deploy/compose/.env.example)"
assert_eq "docs keep pointing at the final release" "$docs_before" "$(cat docs/guides/install.md)"
assert_eq "Cargo.lock and openapi.json are regenerated" "cargo update --workspace|cargo xtask gen-openapi" "$(paste -sd'|' "$CARGO_LOG")"
assert_eq "one commit" "2" "$(commits)"
assert_eq "commit subject" "chore: release v0.1.0-rc.1" "$(git log -1 --format=%s)"
assert_eq "the regenerated files are committed" "" "$(git status --porcelain)"
assert_contains "openapi.json is in the commit" "openapi.json" "$(git show --name-only --format= HEAD)"

echo "== a final bump"
expect_pass "bump to 0.2.0" bump 0.2.0
assert_eq "workspace version" "0.2.0" "$(workspace_version)"
assert_contains "release link" "[v0.2.0 release](https://github.com/ChanningHe/deskhaven/releases/tag/v0.2.0)" "$(cat docs/guides/install.md)"
assert_contains "raw link" "raw.githubusercontent.com/ChanningHe/deskhaven/v0.2.0/deploy/compose" "$(cat docs/guides/install.md)"
assert_contains "another repository's link is untouched" "rustdesk-server/releases/tag/1.1.16" "$(cat docs/guides/install.md)"
assert_contains "a version on a line without a link here is untouched" "Docker Compose v2.20.0" "$(cat docs/guides/install.md)"
assert_contains "a download link and its text, not another version on that line" \
  "Get [v0.2.0 release](https://github.com/ChanningHe/deskhaven/releases/download/v0.2.0/x.tar.gz) (Docker v2.20.0+)" "$(cat docs/guides/install.md)"
assert_contains "a blob link" "ChanningHe/deskhaven/blob/v0.2.0/README.md" "$(cat docs/guides/install.md)"
assert_eq "a link to another site is untouched" "See https://example.com/mirror/v0.1.0/x" "$(cat docs/other.md)"
assert_eq "commit subject" "chore: release v0.2.0" "$(git log -1 --format=%s)"
assert_eq "clean after the bump" "" "$(git status --porcelain)"

echo "== refusals change nothing"
refused() { # LABEL NEEDLE ARGS...
  local label="$1" needle="$2" before
  shift 2
  before="$(commits)"
  expect_fail "$label" "$needle" bump "$@"
  assert_eq "$label: no commit" "$before" "$(commits)"
  assert_eq "$label: version kept" "0.2.0" "$(workspace_version)"
}
refused "no version" "usage"
refused "two versions" "usage" 0.3.0 0.4.0
refused "a leading v" "no leading v" v0.3.0
refused "two components" "is not X.Y.Z" 0.3
refused "an empty pre-release" "is not X.Y.Z" 0.3.0-
refused "a pre-release with a slash" "is not X.Y.Z" 0.3.0-rc/1
refused "the current version" "already 0.2.0" 0.2.0
git tag v0.3.0
refused "a version with a tag" "never reused" 0.3.0
git tag -d v0.3.0 >/dev/null
echo dirt >dirt.txt
refused "an untracked file" "not clean" 0.3.0
rm dirt.txt
echo "# changed" >>Cargo.lock
refused "a modified file" "not clean" 0.3.0
git checkout -q -- Cargo.lock
git checkout -q -b side
refused "another branch" "not on main" 0.3.0
git checkout -q main
git checkout -q --detach
refused "a detached HEAD" "not on main" 0.3.0
git checkout -q main
printf 'DESKHAVEN_TAG=0.2.0\n' >>deploy/compose/.env.example
git commit -q -a -m "test: two tags"
refused "two DESKHAVEN_TAG lines" "exactly one DESKHAVEN_TAG" 0.3.0
git reset -q --hard HEAD~1

echo "== a failing cargo commits nothing and restores the tree"
before="$(commits)"
export STUB_CARGO_FAIL=1
expect_fail "cargo fails" "the working tree was restored" bump 0.3.0
unset STUB_CARGO_FAIL
assert_eq "no commit" "$before" "$(commits)"
assert_eq "the tree is clean" "" "$(git status --porcelain)"
assert_eq "the version is kept" "0.2.0" "$(workspace_version)"
expect_pass "the bump can simply be re-run" bump 0.3.0
assert_eq "re-run version" "0.3.0" "$(workspace_version)"

finish
