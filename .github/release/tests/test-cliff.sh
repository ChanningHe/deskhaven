#!/usr/bin/env bash
# cliff.toml: contributor-controlled subjects are rendered inside code spans (so mentions,
# links, images, HTML and autolinks are inert), and a final release is ONE section even
# when release candidates lie inside its range.
#
# Runs only if git-cliff is installed. The CI job does not install it (a `cargo install`
# takes minutes and the release workflow exercises cliff on every run anyway); run it
# locally, or in the job if a prebuilt git-cliff is on PATH.
set -euo pipefail
# shellcheck source=lib.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
if ! command -v git-cliff >/dev/null 2>&1; then
  echo "skip  test-cliff.sh: git-cliff is not installed"
  exit 0
fi

repo="$TMP/repo"
mkdir -p "$repo"
cd "$repo"
git init -q .
git config user.email t@example.com
git config user.name t
n=0
add() { n=$((n + 1)); echo "$n" >f; git add f; git commit -q -m "$1"; }
cliff() { git-cliff --config "$RELEASE_DIR/cliff.toml" "$@" --strip all 2>/dev/null; }

add "chore: start"
git tag v0.1.0
subjects=(
  'feat: mention @octocat and @org/team and email a@b.example (M2)'
  'fix: link [x](https://evil.example/a) and ![i](https://evil.example/p.png) (M3)'
  'fix: html <img src=x onerror=alert(1)> <script>alert(1)</script> &lt;b&gt; (M4)'
  'fix: backslashes \\ \` \[ \< \@octocat and a trailing slash \ (M5)'
  'fix: bare https://evil.example/path and www.evil.example (M6)'
  'fix: tick ` @x then `` @y and ``` @z ``` and [l](http://e) (M7)'
  'feat: `starts with a tick` and ends with one` (M8)'
  'fix: ![](x) # not heading *bold* _it_ ~~s~~ | a | b | (M10)'
)
for s in "${subjects[@]}"; do add "$s"; done

echo "== subjects are inert code spans"
notes="$(cliff v0.1.0..HEAD --tag v0.2.0)"
for s in "${subjects[@]}"; do
  description="${s#*: }"
  ticks="${description//[^\`]/}"
  fence="$(printf '`%.0s' $(seq 1 $((${#ticks} + 1))))"
  if grep -qF -- "- ${fence} ${description} ${fence} (" <<<"$notes"; then
    ok "rendered as one code span: ${description:0:50}"
  else
    bad "not rendered as the expected code span: ${description}"
  fi
done
assert_eq "no bare mention outside a code span" "0" "$(grep -c -- '^- [^`]' <<<"$notes" || true)"

echo "== a final release is one section"
add "feat: in rc1"
git tag v0.2.0-rc.1
add "fix: in rc2"
git tag v0.2.0-rc.2
add "feat: after rc2"
git tag v0.2.0
sections="$(cliff v0.1.0..v0.2.0 --tag v0.2.0 | grep -c '^## ' || true)"
assert_eq "one section for v0.1.0..v0.2.0 with two rcs inside" "1" "$sections"
assert_contains "...holding the rc work too" "in rc1" "$(cliff v0.1.0..v0.2.0 --tag v0.2.0)"
rc_sections="$(cliff v0.2.0-rc.1..v0.2.0-rc.2 --tag v0.2.0-rc.2 | grep -c '^## ' || true)"
assert_eq "an rc's own notes are one section as well" "1" "$rc_sections"
finish
