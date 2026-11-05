#!/usr/bin/env bash
# Assembles THIRD-PARTY-NOTICES.txt, the notice shipped with every release and copied
# into the deskhaven image:
#   1. every Rust crate linked into the binary (cargo-about; config and template next to
#      this script),
#   2. crates that declare no license (cargo-about cannot render them),
#   3. native libraries and data compiled into the binary (the SecLists password list),
#   4. the console's npm dependencies (optional input, built by web-licenses.mjs).
#
# Usage: .github/release/notices.sh OUTPUT_FILE [WEB_NOTICES_FILE]
# Needs: cargo-about (`.github/ci-setup.sh cargo-about@VERSION/cli`) and network access
# for the crate sources and license data.
set -euo pipefail

output="${1:?usage: notices.sh OUTPUT_FILE [WEB_NOTICES_FILE]}"
web_notices="${2:-}"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
release_dir="$root/.github/release"
passwords_notice="$root/crates/deskhaven-core/data/common-passwords.LICENSE"

# shellcheck source=notices-lib.sh
. "$release_dir/notices-lib.sh"

cd "$root"
generate_crate_notice "$release_dir/about.toml" "$release_dir/about.hbs" "$output"

{
  unlicensed_crates_section
  native_libraries_section

  section "Data compiled into the binary: common password list"
  cat "$passwords_notice"

  if [ -n "$web_notices" ]; then
    section "Web console"
    cat "$web_notices"
  fi
} >>"$output"

require_text "$output" "GNU Affero General Public License" "Apache License" "SecLists" "Daniel Miessler"
if [ -n "$web_notices" ]; then
  require_text "$output" "THIRD-PARTY NOTICES: console dependencies"
fi
echo "wrote $output ($(wc -c <"$output") bytes)"
