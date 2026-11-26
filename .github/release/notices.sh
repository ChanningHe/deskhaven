#!/usr/bin/env bash
# shellcheck source-path=SCRIPTDIR
# Usage: .github/release/notices.sh OUTPUT_FILE [WEB_NOTICES_FILE]
# Needs: cargo-about (`.github/ci-setup.sh cargo-about@VERSION/cli`) and network access.
set -euo pipefail

output="${1:?usage: notices.sh OUTPUT_FILE [WEB_NOTICES_FILE]}"
web_notices="${2:-}"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
release_dir="$root/.github/release"
passwords_notice="$root/crates/deskhaven-core/data/common-passwords.LICENSE"
console_code_notice="$root/web/src/features/remote/THIRD-PARTY-NOTICES.txt"

# shellcheck source=notices-lib.sh
. "$release_dir/notices-lib.sh"

cd "$root"
generate_crate_notice "$release_dir/about.toml" "$release_dir/about.hbs" "$output"

{
  unlicensed_crates_section
  native_libraries_section

  section "Data compiled into the binary: common password list"
  cat "$passwords_notice"

  section "Code compiled into the web console"
  cat "$console_code_notice"

  if [ -n "$web_notices" ]; then
    section "Web console"
    cat "$web_notices"
  fi
} >>"$output"

require_text "$output" "GNU Affero General Public License" "Apache License" "SecLists" "Daniel Miessler" \
  "Nicolas Patry" "Copyright 2008 Google Inc." "Buf Technologies"
if [ -n "$web_notices" ]; then
  require_text "$output" "THIRD-PARTY NOTICES: console dependencies"
fi
echo "wrote $output ($(wc -c <"$output") bytes)"
