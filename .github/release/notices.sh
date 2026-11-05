#!/usr/bin/env bash
# Assembles THIRD-PARTY-NOTICES.txt, the notice shipped with every release and copied
# into the deskhaven image:
#   1. every Rust crate linked into the binary (cargo-about; config and template next to
#      this script),
#   2. crates that declare no license (cargo-about cannot render them),
#   3. data compiled into the binary (the SecLists password list),
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

section() {
  printf '\n\n%s\n%s\n\n' "$(printf '#%.0s' $(seq 1 80))" "$1"
}

cd "$root"
cargo about generate --locked \
  --config "$release_dir/about.toml" \
  --output-file "$output" \
  "$release_dir/about.hbs"

{
  section "Crates without a license declaration"
  cat <<'EOF'
hbb_common and default_net (both from the RustDesk organization) declare no license and
ship no license file, so they appear in no list above. deskhaven treats both as
AGPL-3.0-or-later: for hbb_common this is inferred from rustdesk-server's debian/copyright
("Files: *" under AGPL-3.0, "or any later version"), the tree that carried hbb_common as a
submodule; default_net is assumed to follow the RustDesk client. This is an assumption, not
a grant (see the comments in deny.toml). Sources:
  https://github.com/rustdesk/hbb_common
  https://github.com/rustdesk-org/default_net
EOF

  section "Native libraries linked statically"
  cat <<'EOF'
The crates above wrap, or bundle the sources of, C libraries that end up inside the static
musl binary. Their license texts (Apache-2.0, ISC, MIT) are reproduced in the lists above
under other crates; the copyright holders are:
  OpenSSL 3 (Apache-2.0)      The OpenSSL Project Authors; linked through openssl-sys,
                              which hbb_common's native-tls pulls in
  libsodium (ISC)             Frank Denis and contributors; bundled by libsodium-sys
  SQLite (public domain)      D. Richard Hipp and contributors; bundled by libsqlite3-sys
  musl libc (MIT)             Copyright (c) 2005-2020 Rich Felker, et al.
EOF

  section "Data compiled into the binary: common password list"
  cat "$passwords_notice"

  if [ -n "$web_notices" ]; then
    section "Web console"
    cat "$web_notices"
  fi
} >>"$output"

# Fail loudly if an expected part is missing (a template or config drift would otherwise
# ship a notice that silently lacks it).
for needle in "GNU Affero General Public License" "Apache License" "SecLists" "Daniel Miessler"; do
  grep -q "$needle" "$output" || {
    echo "notices.sh: '$needle' missing from $output" >&2
    exit 1
  }
done
if [ -n "$web_notices" ]; then
  grep -q "THIRD-PARTY NOTICES: console dependencies" "$output" || {
    echo "notices.sh: web section missing from $output" >&2
    exit 1
  }
fi
echo "wrote $output ($(wc -c <"$output") bytes)"
